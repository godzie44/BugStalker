use crate::debugger::{command, Debugger, EventHook, Variable};

pub struct Variables<'a, T: EventHook> {
    dbg: &'a Debugger<T>,
}

impl<'a, T: EventHook> Variables<'a, T> {
    pub fn new(debugger: &'a Debugger<T>) -> Self {
        Self { dbg: debugger }
    }

    pub fn run(&self) -> command::Result<Vec<Variable>> {
        Ok(self.dbg.read_variables()?)
    }
}
