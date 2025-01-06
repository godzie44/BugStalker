mod context;
mod future;
mod tokio;

use crate::debugger::address::Address;
pub use crate::debugger::r#async::future::AsyncFnFutureState;
pub use crate::debugger::r#async::future::Future;
use nix::sys::signal::Signal;
pub use tokio::extract_tokio_version_naive;
pub use tokio::park::BlockThread;
use tokio::task::task_header_state_value_and_ptr;
pub use tokio::worker::Worker;
pub use tokio::TokioVersion;

use crate::debugger::address::RelocatedAddress;
use crate::debugger::error::Error::NoFunctionRanges;
use crate::debugger::error::Error::PlaceNotFound;
use crate::debugger::error::Error::ProcessExit;
use crate::debugger::r#async::context::TokioAnalyzeContext;
use crate::debugger::r#async::future::ParseFutureStateError;
use crate::debugger::r#async::tokio::worker::OwnedList;
use crate::debugger::utils::PopIf;
use crate::debugger::variable::dqe::{Dqe, Selector};
use crate::debugger::{Debugger, Error};
use crate::disable_when_not_stared;
use crate::weak_error;
use nix::unistd::Pid;
use std::rc::Rc;
use tokio::park::try_as_park_thread;
use tokio::worker::try_as_worker;
use super::address::GlobalAddress;
use super::breakpoint::Breakpoint;
use super::debugee::tracer::WatchpointHitType;
use super::register::debug::BreakCondition;
use super::register::debug::BreakSize;

#[derive(Debug, Clone)]
pub struct TaskBacktrace {
    /// Address of the `Header` structure
    pub raw_ptr: RelocatedAddress,
    /// Tokio task id.
    pub task_id: u64,
    /// Futures stack.
    pub futures: Vec<Future>,
}

/// Async backtrace - represent information about current async runtime state.
#[derive(Debug)]
pub struct AsyncBacktrace {
    /// Async workers information.
    pub workers: Vec<Worker>,
    /// Blocking (parked) threads information.
    pub block_threads: Vec<BlockThread>,
    /// Known tasks. Each task has own backtrace, where root is an async function.
    pub tasks: Rc<Vec<TaskBacktrace>>,
}

impl AsyncBacktrace {
    pub fn current_task(&self) -> Option<&TaskBacktrace> {
        let mb_active_block_thread = self.block_threads.iter().find(|t| t.in_focus);
        if let Some(bt) = mb_active_block_thread {
            Some(&bt.bt)
        } else {
            let active_worker = self.workers.iter().find(|t| t.in_focus)?;
            let active_task_id = active_worker.active_task;
            let active_task = if let Some(active_task_id) = active_task_id {
                self.tasks.iter().find(|t| t.task_id == active_task_id)
            } else {
                active_worker.active_task_standby.as_ref()
            }?;
            Some(active_task)
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AsyncError {
    #[error("Backtrace for thread {0} not found")]
    BacktraceShouldExist(Pid),
    #[error("Parse future state: {0}")]
    ParseFutureState(ParseFutureStateError),
    #[error("Incorrect assumption about async runtime: {0}")]
    IncorrectAssumption(&'static str),
    #[error("Current task not found")]
    NoCurrentTaskFound,
    #[error("Async step are impossible cause watchpoint limit is reached (maximum 4 watchpoints), try to remove unused")]
    NotEnoughWatchpointsForStep,
}

/// Result of a async step, if [`SignalInterrupt`] or [`WatchpointInterrupt`] then
/// a step process interrupted and the user should know about it.
/// If `quiet` set to `true` then no hooks should occur.
enum AsyncStepResult {
    Done {
        task_id: u64,
        completed: bool,
    },
    SignalInterrupt {
        signal: Signal,
        quiet: bool,
    },
    WatchpointInterrupt {
        pid: Pid,
        addr: RelocatedAddress,
        ty: WatchpointHitType,
        quiet: bool,
    },
}

impl AsyncStepResult {
    fn signal_interrupt_quiet(signal: Signal) -> Self {
        Self::SignalInterrupt {
            signal,
            quiet: true,
        }
    }

    fn signal_interrupt(signal: Signal) -> Self {
        Self::SignalInterrupt {
            signal,
            quiet: false,
        }
    }

    fn wp_interrupt_quite(pid: Pid, addr: RelocatedAddress, ty: WatchpointHitType) -> Self {
        Self::WatchpointInterrupt {
            pid,
            addr,
            ty,
            quiet: true,
        }
    }

    fn wp_interrupt(pid: Pid, addr: RelocatedAddress, ty: WatchpointHitType) -> Self {
        Self::WatchpointInterrupt {
            pid,
            addr,
            ty,
            quiet: false,
        }
    }
}

impl Debugger {
    pub fn async_backtrace(&mut self) -> Result<AsyncBacktrace, Error> {
        let tokio_version = self.debugee.tokio_version();
        disable_when_not_stared!(self);

        let expl_ctx = self.exploration_ctx().clone();

        let threads = self.debugee.thread_state(&expl_ctx)?;
        let mut analyze_context = TokioAnalyzeContext::new(self, tokio_version.unwrap_or_default());
        let mut backtrace = AsyncBacktrace {
            workers: vec![],
            block_threads: vec![],
            tasks: Rc::new(vec![]),
        };

        let mut tasks = Rc::new(vec![]);

        for thread in threads {
            let worker = weak_error!(try_as_worker(&mut analyze_context, &thread));

            if let Some(Some(w)) = worker {
                // if this is an async worker we need to extract whole future list once
                if tasks.is_empty() {
                    let mut context_initialized_var = analyze_context
                        .debugger()
                        .read_variable(Dqe::Variable(Selector::by_name("CONTEXT", false)))?;
                    let context_initialized = context_initialized_var
                        .pop_if_cond(|results| results.len() == 1)
                        .ok_or(Error::Async(AsyncError::IncorrectAssumption(
                            "CONTEXT not found",
                        )))?;

                    tasks = Rc::new(
                        OwnedList::try_extract(&analyze_context, context_initialized)?
                            .into_iter()
                            .filter_map(|t| weak_error!(t.backtrace()))
                            .collect(),
                    );
                    backtrace.tasks = tasks.clone();
                }

                backtrace.workers.push(w);
            } else {
                // maybe thread block on future?
                let thread = weak_error!(try_as_park_thread(&mut analyze_context, &thread));
                if let Some(Some(pt)) = thread {
                    backtrace.block_threads.push(pt);
                }
            }
        }

        self.expl_ctx_swap(expl_ctx);

        Ok(backtrace)
    }

    pub fn async_step_over(&mut self) -> Result<(), Error> {
        disable_when_not_stared!(self);
        self.expl_ctx_restore_frame()?;

        match self.async_step_over_any()? {
            AsyncStepResult::Done { task_id, completed } => {
                self.execute_on_async_step_hook(task_id, completed)?
            }
            AsyncStepResult::SignalInterrupt { signal, quiet } if !quiet => {
                self.hooks.on_signal(signal);
            }
            AsyncStepResult::WatchpointInterrupt {
                pid,
                addr,
                ref ty,
                quiet,
            } if !quiet => self.execute_on_watchpoint_hook(pid, addr, ty)?,
            _ => {}
        };
        Ok(())
    }

    /// Do debugee async step (over subroutine calls too and always check that current task doesn't change).
    /// Returns [`StepResult::SignalInterrupt`] if step is interrupted by a signal,
    /// [`StepResult::WatchpointInterrupt`] if step is interrupted by a watchpoint,
    /// or [`StepResult::Done`] if step done or task completed.
    ///
    /// **! change exploration context**
    fn async_step_over_any(&mut self) -> Result<AsyncStepResult, Error> {
        let ctx = self.exploration_ctx();
        let mut current_location = ctx.location();

        let async_bt = self.async_backtrace()?;
        let current_task = async_bt
            .current_task()
            .ok_or(AsyncError::NoCurrentTaskFound)?;
        let task_id = current_task.task_id;

        // take _task_context local variable, used to determine (fast path)
        // what task we ended up after trying to take a step
        let initial_task_context = self
            .read_variable(Dqe::Variable(Selector::by_name("_task_context", true)))?
            .pop_if_cond(|results| results.len() == 1)
            .and_then(|t_ctx| t_ctx.into_value().into_raw_ptr())
            .and_then(|ptr| ptr.value)
            .ok_or_else(|| {
                AsyncError::IncorrectAssumption("`_task_context` local variable should exist")
            })? as usize;

        let task_ptr = current_task.raw_ptr;

        let future_is_waiter = |f: &Future| {
            if let Future::TokioJoinHandleFuture(jh_f) = f {
                return jh_f.wait_for_task == task_ptr;
            }
            return false;
        };

        let waiter_found = async_bt
            .tasks
            .iter()
            .flat_map(|t| t.futures.iter())
            .chain(
                async_bt
                    .block_threads
                    .iter()
                    .flat_map(|thread| thread.bt.futures.iter()),
            )
            .any(future_is_waiter);

        // if waiter found - set watchpoint at task completion flag
        let waiter_wp = if waiter_found {
            let (_, state_ptr) = task_header_state_value_and_ptr(self, task_ptr)?;
            let state_addr = RelocatedAddress::from(state_ptr);
            Some(
                self.set_watchpoint_on_memory(
                    state_addr,
                    BreakSize::Bytes8,
                    BreakCondition::DataWrites,
                    true,
                )?
                .to_owned(),
            )
        } else {
            None
        };

        // determine current function, if no debug information for function - step until function found
        let func = loop {
            let dwarf = &self.debugee.debug_info(current_location.pc)?;
            // step's stop only if there is debug information for PC and current function can be determined
            if let Ok(Some(func)) = dwarf.find_function_by_pc(current_location.global_pc) {
                break func;
            }
            match self.single_step_instruction()? {
                Some(super::StopReason::SignalStop(_, sign)) => {
                    return Ok(AsyncStepResult::signal_interrupt(sign));
                }
                Some(super::StopReason::Watchpoint(pid, addr, ty)) => {
                    return Ok(AsyncStepResult::wp_interrupt(pid, addr, ty));
                }
                _ => {}
            }
            current_location = self.exploration_ctx().location();
        };

        let prolog = func.prolog()?;
        let dwarf = &self.debugee.debug_info(current_location.pc)?;
        let inline_ranges = func.inline_ranges();

        let current_place = dwarf
            .find_place_from_pc(current_location.global_pc)?
            .ok_or(PlaceNotFound(current_location.global_pc))?;

        let mut step_over_breakpoints = vec![];
        let mut to_delete = vec![];

        let mut task_completed = false;
        let fn_full_name = func.full_name();
        for range in func.ranges() {
            let mut place = func
                .unit()
                .find_place_by_pc(GlobalAddress::from(range.begin))
                .ok_or_else(|| NoFunctionRanges(fn_full_name.clone()))?;

            while place.address.in_range(range) {
                // skip places in function prolog
                if place.address.in_range(&prolog) {
                    match place.next() {
                        None => break,
                        Some(n) => place = n,
                    }
                    continue;
                }

                // guard against a step at inlined function body
                let in_inline_range = place.address.in_ranges(&inline_ranges);

                if !in_inline_range
                    && place.is_stmt
                    && place.address != current_place.address
                    && place.line_number != current_place.line_number
                {
                    let load_addr = place
                        .address
                        .relocate_to_segment_by_pc(&self.debugee, current_location.pc)?;
                    if self.breakpoints.get_enabled(load_addr).is_none() {
                        step_over_breakpoints.push(load_addr);
                        to_delete.push(load_addr);
                    }
                }

                match place.next() {
                    None => break,
                    Some(n) => place = n,
                }
            }
        }

        step_over_breakpoints
            .into_iter()
            .try_for_each(|load_addr| {
                self.breakpoints
                    .add_and_enable(Breakpoint::new_temporary_async(
                        dwarf.pathname(),
                        load_addr,
                        current_location.pid,
                    ))
                    .map(|_| ())
            })?;

        loop {
            let stop_reason = self.continue_execution()?;
            // hooks already called at [`Self::continue_execution`], so use `quite` opt
            match stop_reason {
                super::StopReason::SignalStop(_, sign) => {
                    to_delete.into_iter().try_for_each(|addr| {
                        self.remove_breakpoint(Address::Relocated(addr)).map(|_| ())
                    })?;
                    if let Some(wp) = waiter_wp {
                        self.remove_watchpoint_by_addr(wp.address)?;
                    }
                    return Ok(AsyncStepResult::signal_interrupt_quiet(sign));
                }
                super::StopReason::Watchpoint(pid, current_pc, ty) => {
                    let is_tmp_wp = if let WatchpointHitType::DebugRegister(ref reg) = ty {
                        self.watchpoints
                            .all()
                            .iter()
                            .any(|wp| wp.register() == Some(*reg) && wp.is_temporary())
                    } else {
                        false
                    };

                    if is_tmp_wp {
                        // taken from tokio sources
                        const COMPLETE: usize = 0b0010;
                        let (value, _) = task_header_state_value_and_ptr(self, task_ptr)?;

                        if value & COMPLETE == COMPLETE {
                            task_completed = true;
                            break;
                        } else {
                            continue;
                        }
                    } else {
                        to_delete.into_iter().try_for_each(|addr| {
                            self.remove_breakpoint(Address::Relocated(addr)).map(|_| ())
                        })?;
                        if let Some(wp) = waiter_wp {
                            self.remove_watchpoint_by_addr(wp.address)?;
                        }

                        return Ok(AsyncStepResult::wp_interrupt_quite(pid, current_pc, ty));
                    }
                }
                _ => {}
            }

            // check that _task_context equals to initial
            let mb_task_context = self
                .read_variable(Dqe::Variable(Selector::by_name("_task_context", true)))?
                .pop_if_cond(|results| results.len() == 1)
                .and_then(|t_ctx| t_ctx.into_value().into_raw_ptr())
                .and_then(|ptr| ptr.value)
                .map(|t_ctx| t_ctx as usize);

            let context_equals = if let Some(task_context) = mb_task_context {
                task_context as usize == initial_task_context
            } else {
                false
            };

            if context_equals {
                // fast path
                break;
            }

            // check that task are still exists in the runtime
            let async_bt = self.async_backtrace()?;
            if !async_bt.tasks.iter().any(|t| t.task_id == task_id) {
                // initial task not found, end step
                break;
            }

            let current_task = async_bt.current_task();
            if current_task.map(|t| t.task_id) == Some(task_id) {
                break;
            }

            // wait until next break are hits
        }

        to_delete
            .into_iter()
            .try_for_each(|addr| self.remove_breakpoint(Address::Relocated(addr)).map(|_| ()))?;

        if let Some(wp) = waiter_wp {
            self.remove_watchpoint_by_addr(wp.address)?;
        }

        if self.debugee.is_exited() {
            // todo add exit code here
            return Err(ProcessExit(0));
        }

        self.expl_ctx_update_location()?;
        Ok(AsyncStepResult::Done {
            task_id,
            completed: task_completed,
        })
    }
}
