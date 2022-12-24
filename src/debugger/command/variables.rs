use crate::debugger::{command, Debugger, EventHook, GenericVariable};

pub struct Variables<'a, T: EventHook> {
    dbg: &'a Debugger<T>,
}

impl<'a, T: EventHook> Variables<'a, T> {
    pub fn new(debugger: &'a Debugger<T>) -> Self {
        Self { dbg: debugger }
    }

    pub fn run(&self) -> command::Result<Vec<GenericVariable<T>>> {
        Ok(self.dbg.read_variables()?)
    }
}
