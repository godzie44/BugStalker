use crate::debugger::{command, uw, Debugger, EventHook};

pub struct Backtrace<'a, T: EventHook> {
    dbg: &'a Debugger<T>,
}

impl<'a, T: EventHook> Backtrace<'a, T> {
    pub fn new(debugger: &'a Debugger<T>) -> Self {
        Self { dbg: debugger }
    }

    pub fn run(&self) -> command::Result<uw::Backtrace> {
        Ok(self.dbg.backtrace()?)
    }
}
