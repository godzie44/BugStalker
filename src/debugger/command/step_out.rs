use crate::debugger::{command, Debugger, EventHook};

// Execute until selected stack frame returns
pub struct StepOut<'a, T: EventHook> {
    dbg: &'a Debugger<'a, T>,
}

impl<'a, T: EventHook> StepOut<'a, T> {
    pub fn new(debugger: &'a Debugger<'a, T>) -> Self {
        Self { dbg: debugger }
    }

    pub fn run(&self) -> command::Result<()> {
        Ok(self.dbg.step_out()?)
    }
}
