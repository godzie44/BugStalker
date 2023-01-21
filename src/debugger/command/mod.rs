mod backtrace;
mod r#break;
mod r#continue;
mod frame;
mod memory;
mod quit;
mod register;
mod run;
mod step_instruction;
mod step_into;
mod step_out;
mod step_over;
mod symbol;
mod trace;
mod variables;

pub use backtrace::Backtrace;
pub use frame::Frame;
pub use memory::Memory;
pub use quit::Quit;
pub use r#break::Break;
pub use r#break::Breakpoint as BreakpointType;
pub use r#continue::Continue;
pub use register::Register;
pub use run::Run;
pub use step_instruction::StepI;
pub use step_into::StepInto;
pub use step_out::StepOut;
pub use step_over::StepOver;
pub use symbol::Symbol;
pub use trace::Trace;
pub use variables::Variables;

use std::result;

#[derive(thiserror::Error, Debug)]
pub enum CommandError {
    #[error("invalid command arguments (see help `command`)")]
    InvalidArguments,
    #[error("invalid command arguments (see help `command`): {0}")]
    InvalidArgumentsEx(String),
    #[error(transparent)]
    Debugger(#[from] anyhow::Error),
}

pub type Result<T> = result::Result<T, CommandError>;

pub mod helper {
    use crate::debugger::command;
    use crate::debugger::command::CommandError;

    pub fn check_args_count(args: &Vec<&str>, min_expected_count: usize) -> command::Result<()> {
        if args.len() < min_expected_count {
            return Err(CommandError::InvalidArguments);
        }
        Ok(())
    }
}
