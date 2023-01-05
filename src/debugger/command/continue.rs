use crate::debugger::{command, Debugger, EventHook};
use anyhow::Context;

pub struct Continue<'a, T: EventHook> {
    dbg: &'a mut Debugger<T>,
}

impl<'a, T: EventHook> Continue<'a, T> {
    pub fn new(debugger: &'a mut Debugger<T>) -> Self {
        Self { dbg: debugger }
    }

    pub fn run(&mut self) -> command::Result<()> {
        Ok(self
            .dbg
            .continue_execution()
            .context("Failed to continue execution")?)
    }
}
