use super::task::Task;
use crate::debugger::address::RelocatedAddress;
use crate::debugger::r#async::context::TokioAnalyzeContext;
use crate::debugger::r#async::{AsyncError, TaskBacktrace};
use crate::debugger::utils::PopIf;
use crate::debugger::variable::dqe::{Dqe, Selector};
use crate::debugger::variable::value::Value;
use crate::debugger::{Error, ThreadSnapshot, Tracee};

/// Represent a thread that blocks on future execution (`tokio::task::spawn_blocking` for example how to create this thread).
#[derive(Debug, Clone)]
pub struct BlockThread {
    /// A thread that block on future.
    pub thread: Tracee,
    /// A futures backtrace.
    pub bt: TaskBacktrace,
    /// True if thread in focus. This how `bs` choose an "active worker".
    pub in_focus: bool,
}

/// If thread `thread` is block on than return it, return `Ok(None)` if it's not.
pub fn try_as_park_thread(
    context: &mut TokioAnalyzeContext,
    thread: &ThreadSnapshot,
) -> Result<Option<BlockThread>, Error> {
    let backtrace = thread
        .bt
        .as_ref()
        .ok_or(AsyncError::BacktraceShouldExist(thread.thread.pid))?;

    let Some(block_on_frame_num) = backtrace.iter().position(|frame| {
        let Some(fn_name) = frame.func_name.as_ref() else {
            return false;
        };
        fn_name.contains("CachedParkThread::block_on")
            && !fn_name.contains("CachedParkThread::block_on::")
    }) else {
        return Ok(None);
    };

    let debugger = context.debugger_mut();
    debugger.expl_ctx_switch_thread(thread.thread.pid)?;
    debugger.set_frame_into_focus(block_on_frame_num as u32)?;

    let future = debugger
        .read_variable(Dqe::Variable(Selector::by_name("f", true)))?
        .pop_if_single_el()
        .ok_or(AsyncError::IncorrectAssumption(
            "it looks like it's a park thread, but variable `f` not found at `block_on` fn",
        ))?;

    let Some(Value::RustEnum(fut)) = future.value else {
        return Err(Error::Async(AsyncError::IncorrectAssumption(
            "it looks like it's a park thread, but variable `f` not a future",
        )));
    };
    let task = Task::from_enum_repr(
        RelocatedAddress::from(fut.raw_address.unwrap_or_default()),
        0,
        fut,
    );

    Ok(Some(BlockThread {
        thread: thread.thread.clone(),
        in_focus: thread.in_focus,
        bt: task.backtrace()?,
    }))
}
