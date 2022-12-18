use crate::debugger::{command, Debugger, EventHook};

/// Step program until it reaches a different source line.
pub struct StepInto<'a, T: EventHook> {
    dbg: &'a Debugger<T>,
}

impl<'a, T: EventHook> StepInto<'a, T> {
    pub fn new(debugger: &'a Debugger<T>) -> Self {
        Self { dbg: debugger }
    }

    pub fn run(&self) -> command::Result<()> {
        Ok(self.dbg.step_into()?)
    }
}
