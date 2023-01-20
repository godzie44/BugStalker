use crate::debugger::{command, Debugger};

// Execute until selected stack frame returns
pub struct StepOut<'a> {
    dbg: &'a mut Debugger,
}

impl<'a> StepOut<'a> {
    pub fn new(debugger: &'a mut Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn run(&mut self) -> command::Result<()> {
        Ok(self.dbg.step_out()?)
    }
}
