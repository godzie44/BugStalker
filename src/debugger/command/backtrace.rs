use crate::debugger::{command, uw, Debugger};

pub struct Backtrace<'a> {
    dbg: &'a Debugger,
}

impl<'a> Backtrace<'a> {
    pub fn new(debugger: &'a Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&self) -> command::HandleResult<uw::Backtrace> {
        Ok(self.dbg.backtrace(self.dbg.debugee.thread_in_focus())?)
    }
}
