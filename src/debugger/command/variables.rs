use crate::debugger::variable::VariableIR;
use crate::debugger::{command, Debugger, EventHook};

pub struct Variables<'a, T: EventHook> {
    dbg: &'a Debugger<T>,
}

impl<'a, T: EventHook> Variables<'a, T> {
    pub fn new(debugger: &'a Debugger<T>) -> Self {
        Self { dbg: debugger }
    }

    pub fn run(&mut self) -> command::Result<Vec<VariableIR>> {
        Ok(self.dbg.read_variables()?)
    }
}
