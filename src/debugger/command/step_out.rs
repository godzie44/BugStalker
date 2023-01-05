use crate::debugger::{command, Debugger, EventHook};

// Execute until selected stack frame returns
pub struct StepOut<'a, T: EventHook> {
    dbg: &'a mut Debugger<T>,
}

impl<'a, T: EventHook> StepOut<'a, T> {
    pub fn new(debugger: &'a mut Debugger<T>) -> Self {
        Self { dbg: debugger }
    }

    pub fn run(&mut self) -> command::Result<()> {
        Ok(self.dbg.step_out()?)
    }
}
