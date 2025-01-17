use crate::debugger::r#async::AsyncBacktrace;
use crate::debugger::{Debugger, Error};

#[derive(Debug, Clone)]
pub enum Command {
    ShortBacktrace,
    FullBacktrace,
    CurrentTask(Option<String>),
    StepOver,
    StepOut,
}

pub enum AsyncCommandResult<'a> {
    StepOver,
    StepOut,
    ShortBacktrace(AsyncBacktrace),
    FullBacktrace(AsyncBacktrace),
    CurrentTask(AsyncBacktrace, Option<&'a str>),
}

pub struct Handler<'a> {
    dbg: &'a mut Debugger,
}

impl<'a> Handler<'a> {
    pub fn new(debugger: &'a mut Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle<'cmd>(&mut self, cmd: &'cmd Command) -> Result<AsyncCommandResult<'cmd>, Error> {
        let result = match cmd {
            Command::ShortBacktrace => {
                AsyncCommandResult::ShortBacktrace(self.dbg.async_backtrace()?)
            }
            Command::FullBacktrace => {
                AsyncCommandResult::FullBacktrace(self.dbg.async_backtrace()?)
            }
            Command::CurrentTask(regex) => {
                AsyncCommandResult::CurrentTask(self.dbg.async_backtrace()?, regex.as_deref())
            }
            Command::StepOver => {
                self.dbg.async_step_over()?;
                AsyncCommandResult::StepOver
            }
            Command::StepOut => {
                self.dbg.async_step_out()?;
                AsyncCommandResult::StepOut
            }
        };
        Ok(result)
    }
}
