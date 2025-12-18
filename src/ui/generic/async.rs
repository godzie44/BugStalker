use crate::debugger::r#async::AsyncBacktrace;
use crate::debugger::r#async::AsyncFnFutureState;
use crate::debugger::r#async::Future;
use crate::debugger::r#async::TaskBacktrace;
use crate::ui::generic::print::ExternalPrinter;
use crate::ui::generic::print::style::{
    AsyncTaskView, ErrorView, FutureFunctionView, FutureTypeView,
};
use crossterm::style::Stylize;
use nix::errno::Errno;
use nix::libc;
use nix::sys::time::TimeSpec;
use std::mem::MaybeUninit;
use std::ops::Sub;
use std::time::Duration;

fn print_future(backtrace: &AsyncBacktrace, num: u32, future: &Future, printer: &ExternalPrinter) {
    match future {
        Future::AsyncFn(fn_fut) => {
            printer.println(format!(
                "#{num} async fn {}",
                FutureFunctionView::from(&fn_fut.async_fn)
            ));
            match fn_fut.state {
                AsyncFnFutureState::Suspend(await_num) => {
                    printer.println(format!("\tsuspended at await point {await_num}"));
                }
                AsyncFnFutureState::Panicked => {
                    printer.println("\tpanicked!");
                }
                AsyncFnFutureState::Returned => {
                    printer.println("\talready resolved");
                }
                AsyncFnFutureState::Unresumed => {
                    printer.println("\tjust created");
                }
                AsyncFnFutureState::Ok => {
                    printer.println("\tcompleted");
                }
            }
        }
        Future::Custom(custom_fut) => {
            printer.println(format!(
                "#{num} future {}",
                FutureTypeView::from(custom_fut.name.to_string())
            ));
        }
        Future::TokioJoinHandleFuture(jh_fut) => {
            let wait_for = backtrace
                .tasks
                .iter()
                .find(|t| t.raw_ptr == jh_fut.wait_for_task);
            let wait_for_str = wait_for
                .map(|task| format!(", wait for task id={}", task.task_id))
                .unwrap_or_default();

            printer.println(format!(
                "#{num} Join future {}{}",
                FutureTypeView::from(jh_fut.name.to_string()),
                wait_for_str,
            ));
        }
        Future::TokioSleep(sleep_fut) => {
            fn now_timespec() -> Result<TimeSpec, Errno> {
                let mut t = MaybeUninit::uninit();
                let res = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, t.as_mut_ptr()) };
                if res == -1 {
                    return Err(Errno::last());
                }
                let t = unsafe { t.assume_init() };
                Ok(TimeSpec::new(t.tv_sec, t.tv_nsec))
            }

            pub fn diff_from_now(i: (i64, u32)) -> (std::cmp::Ordering, Duration) {
                let now = now_timespec().expect("broken system clock");
                let this = TimeSpec::new(i.0, i.1 as i64);
                if this < now {
                    (std::cmp::Ordering::Less, Duration::from(now.sub(this)))
                } else {
                    (std::cmp::Ordering::Greater, Duration::from(this.sub(now)))
                }
            }

            let render = match diff_from_now(sleep_fut.instant) {
                (std::cmp::Ordering::Less, d) => {
                    format!("already happened {} seconds ago ", d.as_secs())
                }
                (std::cmp::Ordering::Greater, d) => {
                    format!("{} seconds from now", d.as_secs())
                }
                _ => unreachable!(),
            };

            printer.println(format!("#{num} sleep future, sleeping {render}",));
        }
        Future::UnknownFuture => {
            printer.println(format!("#{num} undefined future",));
        }
    }
}

fn print_task(backtrace: &AsyncBacktrace, task: &TaskBacktrace, printer: &ExternalPrinter) {
    let task_descr = format!("Task id: {}", task.task_id).bold();
    printer.println(AsyncTaskView::from(task_descr));

    for (i, fut) in task.futures.iter().enumerate() {
        print_future(backtrace, i as u32, fut, printer);
    }
}

pub fn print_backtrace(backtrace: &AsyncBacktrace, printer: &ExternalPrinter) {
    let mut workers = backtrace.workers.clone();
    let mut block_threads = backtrace.block_threads.clone();
    workers.sort_by_key(|w| w.thread.number);
    block_threads.sort_by_key(|pt| pt.thread.number);

    for bt in &block_threads {
        let block_thread_header = format!(
            "Thread #{} (pid: {}) block on:",
            bt.thread.number, bt.thread.pid,
        );
        if bt.in_focus {
            printer.println(block_thread_header.bold());
        } else {
            printer.println(block_thread_header);
        }

        for (i, fut) in bt.bt.futures.iter().enumerate() {
            print_future(backtrace, i as u32, fut, printer);
        }
    }

    printer.println("");

    for worker in &workers {
        let worker_header = format!(
            "Async worker #{} (pid: {}, local queue length: {})",
            worker.thread.number,
            worker.thread.pid,
            worker.queue.len(),
        );
        if worker.in_focus {
            printer.println(worker_header.bold());
        } else {
            printer.println(worker_header);
        }

        if let Some(active_task_idx) = worker.active_task {
            let active_task = backtrace
                .tasks
                .get(active_task_idx as usize)
                .or(worker.active_task_standby.as_ref());

            if let Some(active_task) = active_task {
                let task_descr = format!("Active task: {}", active_task.task_id).bold();
                printer.println(AsyncTaskView::from(task_descr));

                for (i, fut) in active_task.futures.iter().enumerate() {
                    print_future(backtrace, i as u32, fut, printer);
                }
            }
        }
    }
}

pub fn print_backtrace_full(backtrace: &AsyncBacktrace, printer: &ExternalPrinter) {
    print_backtrace(backtrace, printer);

    printer.println("");
    printer.println("Known tasks:");

    for task in backtrace.tasks.iter() {
        print_task(backtrace, task, printer);
    }
}

pub fn print_task_ex(backtrace: &AsyncBacktrace, printer: &ExternalPrinter, regex: Option<&str>) {
    if let Some(regex) = regex {
        let re = regex::Regex::new(regex).unwrap();

        let tasks = &backtrace.tasks;
        for task in tasks.iter() {
            if let Some(Future::AsyncFn(f)) = task.futures.first()
                && re.find(&f.async_fn).is_some()
            {
                print_task(backtrace, task, printer);
            }
        }
    } else {
        // print current task
        let Some(active_task) = backtrace.current_task() else {
            printer.println(ErrorView::from(
                "no active task found for current worker, or no active worker found",
            ));
            return;
        };

        print_task(backtrace, active_task, printer);
    }
}
