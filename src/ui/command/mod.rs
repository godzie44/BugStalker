//! An interface to a debugger.
//! This is the most preferred way to use a debugger functional from UI layer.
//!
//! Contains commands and corresponding command handlers. Command is a some sort of request to
//! debugger that defines an action and a list of input arguments.

pub mod r#async;
pub mod backtrace;
pub mod r#break;
pub mod call;
pub mod r#continue;
pub mod frame;
pub mod memory;
pub mod parser;
pub mod print;
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
pub mod trigger;
pub mod watch;

use crate::debugger::Error;

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
    Print(print::Command),
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
    Async(r#async::Command),
    Trigger(trigger::Command),
    Call(call::Command),
    Help {
        command: Option<String>,
        reason: Option<String>,
    },
}
