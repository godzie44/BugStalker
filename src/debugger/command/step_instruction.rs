use crate::debugger::{command, Debugger};

/// Step on next instruction
pub struct StepI<'a> {
    dbg: &'a Debugger,
}

impl<'a> StepI<'a> {
    pub fn new(debugger: &'a Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn run(&mut self) -> command::Result<()> {
        Ok(self.dbg.stepi()?)
    }
}
