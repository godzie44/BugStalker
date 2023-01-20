use crate::debugger::{command, Debugger};

/// Step program until it reaches a different source line.
pub struct StepInto<'a> {
    dbg: &'a Debugger,
}

impl<'a> StepInto<'a> {
    pub fn new(debugger: &'a Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn run(&self) -> command::Result<()> {
        Ok(self.dbg.step_into()?)
    }
}
