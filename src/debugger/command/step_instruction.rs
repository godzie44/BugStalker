use crate::debugger::{command, Debugger, EventHook};

/// Step on next instruction
pub struct StepI<'a, T: EventHook> {
    dbg: &'a Debugger<T>,
}

impl<'a, T: EventHook> StepI<'a, T> {
    pub fn new(debugger: &'a Debugger<T>) -> Self {
        Self { dbg: debugger }
    }

    pub fn run(&self) -> command::Result<()> {
        Ok(self.dbg.stepi()?)
    }
}
