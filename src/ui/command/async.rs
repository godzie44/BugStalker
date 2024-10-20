use crate::debugger::r#async::AsyncBacktrace;
use crate::debugger::{Debugger, Error};

#[derive(Debug, Clone)]
pub enum Command {
    ShortBacktrace,
    FullBacktrace,
    CurrentTask(Option<String>),
}

pub struct Handler<'a> {
    dbg: &'a mut Debugger,
}

impl<'a> Handler<'a> {
    pub fn new(debugger: &'a mut Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&mut self, cmd: &Command) -> Result<AsyncBacktrace, Error> {
        let result = match cmd {
            Command::ShortBacktrace => self.dbg.async_backtrace()?,
            Command::FullBacktrace => self.dbg.async_backtrace()?,
            Command::CurrentTask(_) => self.dbg.async_backtrace()?,
        };
        Ok(result)
    }
}
