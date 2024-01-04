use crate::common::DebugeeRunInfo;
use crate::common::TestHooks;
use crate::prepare_debugee_process;
use crate::{assert_no_proc, MT_APP};
use bugstalker::debugger::unwind::Backtrace;
use bugstalker::debugger::Debugger;
use itertools::Itertools;
use serial_test::serial;
use std::ffi::OsStr;

#[test]
#[serial]
fn test_multithreaded_app_running() {
    let process = prepare_debugee_process(MT_APP, &[]);
    let debugee_pid = process.pid();
    let mut debugger = Debugger::new(process, TestHooks::default(), vec![]).unwrap();
    debugger.start_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_multithreaded_breakpoints() {
    let process = prepare_debugee_process(MT_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone()), vec![]).unwrap();

    // set breakpoint at program start.
    debugger.set_breakpoint_at_line("mt.rs", 6).unwrap();
    // set breakpoints at thread 1 code.
    debugger.set_breakpoint_at_line("mt.rs", 24).unwrap();
    // set breakpoint at thread 2 code.
    debugger.set_breakpoint_at_line("mt.rs", 36).unwrap();
    // set breakpoint at program ends.
    debugger.set_breakpoint_at_line("mt.rs", 14).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(6));
    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(36));
    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(24));
    debugger.continue_debugee().unwrap();
    assert_eq!(info.line.take(), Some(14));

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

fn backtrace_contains_fn(backtrace: &Backtrace, f_name: &str) -> bool {
    backtrace.iter().any(|frame| {
        frame
            .func_name
            .as_ref()
            .map(|f| f.contains(f_name))
            .unwrap_or(false)
    })
}

#[test]
#[serial]
fn test_multithreaded_backtrace() {
    let process = prepare_debugee_process(MT_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone()), vec![]).unwrap();

    debugger.set_breakpoint_at_line("mt.rs", 24).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(24));

    let threads = debugger.thread_state().unwrap();
    let current_thread = threads.into_iter().find(|t| t.in_focus).unwrap();

    let current_place = current_thread.place.unwrap();
    assert!(current_place.file.iter().contains(&OsStr::new("mt.rs")));
    assert_eq!(current_place.line_number, 24);

    let bt = debugger.backtrace(current_thread.thread.pid).unwrap();
    assert_eq!(bt[0].func_name.as_ref().unwrap(), "mt::sum1");
    assert!(backtrace_contains_fn(&bt, "new::thread_start"));

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}

#[test]
#[serial]
fn test_multithreaded_trace() {
    let process = prepare_debugee_process(MT_APP, &[]);
    let debugee_pid = process.pid();
    let info = DebugeeRunInfo::default();
    let mut debugger = Debugger::new(process, TestHooks::new(info.clone()), vec![]).unwrap();

    debugger.set_breakpoint_at_line("mt.rs", 23).unwrap();

    debugger.start_debugee().unwrap();
    assert_eq!(info.line.take(), Some(23));

    let trace = debugger.thread_state().unwrap();
    assert_eq!(trace.len(), 5);

    trace
        .iter()
        .any(|thread| backtrace_contains_fn(thread.bt.as_ref().unwrap(), "mt::main"));
    trace
        .iter()
        .any(|thread| backtrace_contains_fn(thread.bt.as_ref().unwrap(), "std::thread::sleep"));
    trace
        .iter()
        .any(|thread| backtrace_contains_fn(thread.bt.as_ref().unwrap(), "mt::sum2"));
    trace
        .iter()
        .any(|thread| backtrace_contains_fn(thread.bt.as_ref().unwrap(), "mt::sum3"));

    debugger.continue_debugee().unwrap();
    assert_no_proc!(debugee_pid);
}
