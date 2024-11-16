mod context;
mod future;
mod tokio;

pub use crate::debugger::r#async::future::AsyncFnFutureState;
pub use crate::debugger::r#async::future::Future;
pub use tokio::extract_tokio_version_naive;
pub use tokio::park::BlockThread;
pub use tokio::worker::Worker;
pub use tokio::TokioVersion;

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

#[derive(Debug)]
pub struct TaskBacktrace {
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

#[derive(Debug, thiserror::Error)]
pub enum AsyncError {
    #[error("Backtrace for thread {0} not found")]
    BacktraceShouldExist(Pid),
    #[error("Parse future state: {0}")]
    ParseFutureState(ParseFutureStateError),
    #[error("Incorrect assumption about async runtime: {0}")]
    IncorrectAssumption(&'static str),
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
}
