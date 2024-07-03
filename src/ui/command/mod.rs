//! An interface to a debugger.
//! This is the most preferred way to use a debugger functional from UI layer.
//!
//! Contains commands and corresponding command handlers. Command is a some sort of request to
//! debugger that define an action and a list of input arguments. Command handler validate command,
//! define what exactly debugger must to do and return result of it.

pub mod arguments;
pub mod backtrace;
pub mod r#break;
pub mod r#continue;
pub mod frame;
pub mod memory;
pub mod parser;
pub mod register;
pub mod run;
pub mod sharedlib;
pub mod source_code;
pub mod step_instruction;
pub mod step_into;
pub mod step_out;
pub mod step_over;
pub mod symbol;
pub mod thread;
pub mod variables;
pub mod watch;

use crate::debugger::variable::dqe::Dqe;
use crate::debugger::Error;
use crate::debugger::variable::select::DQE;

#[derive(thiserror::Error, Debug)]
pub enum CommandError {
    #[error("malformed command")]
    Parsing(String),
    #[error("render error: \n{0}")]
    FileRender(anyhow::Error),
    #[error(transparent)]
    Handle(#[from] Error),
}

pub type CommandResult<T> = Result<T, CommandError>;

/// External commands that can be processed by the debugger.
#[derive(Debug, Clone)]
pub enum Command {
    PrintVariables(Dqe),
    PrintArguments(Dqe),
    PrintBacktrace(backtrace::Command),
    Continue,
    Frame(frame::Command),
    Run,
    StepInstruction,
    StepInto,
    StepOut,
    StepOver,
    PrintSymbol(String),
    Breakpoint(r#break::Command),
    Watchpoint(watch::Command),
    Memory(memory::Command),
    Register(register::Command),
    Thread(thread::Command),
    SharedLib,
    SourceCode(source_code::Command),
    SkipInput,
    Oracle(String, Option<String>),
    Help {
        command: Option<String>,
        reason: Option<String>,
    },
}
