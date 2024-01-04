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
pub mod disasm;
pub mod frame;
pub mod memory;
pub mod parser;
pub mod register;
pub mod run;
pub mod sharedlib;
pub mod step_instruction;
pub mod step_into;
pub mod step_out;
pub mod step_over;
pub mod symbol;
pub mod thread;
pub mod variables;

use crate::debugger::variable::select::Expression;
use crate::debugger::Error;

#[derive(thiserror::Error, Debug)]
pub enum CommandError {
    #[error("malformed command (try `help command`):\n{0}")]
    Parsing(anyhow::Error),
    #[error(transparent)]
    Handle(#[from] Error),
}

pub type CommandResult<T> = Result<T, CommandError>;

/// External commands that can be processed by the debugger.
#[derive(Debug)]
pub enum Command {
    PrintVariables(Expression),
    PrintArguments(Expression),
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
    Memory(memory::Command),
    Register(register::Command),
    Thread(thread::Command),
    SharedLib,
    DisAsm,
    SkipInput,
    Oracle(String, Option<String>),
    Help {
        command: Option<String>,
        reason: Option<String>,
    },
}
