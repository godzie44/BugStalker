use crate::debugger::dwarf::parse::Place;
use crate::debugger::{command, Debugger, EventHook};

/// Step on next instruction
pub struct StepI<'a, T: EventHook> {
    dbg: &'a Debugger<'a, T>,
}

impl<'a, T: EventHook> StepI<'a, T> {
    pub fn new(debugger: &'a Debugger<'a, T>) -> Self {
        Self { dbg: debugger }
    }

    pub fn run(&self) -> command::Result<Option<Place>> {
        Ok(self.dbg.stepi()?)
    }
}
