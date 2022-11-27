use crate::debugger::{command, Debugger, EventHook};
use anyhow::Context;

pub struct Continue<'a, T: EventHook> {
    dbg: &'a Debugger<T>,
}

impl<'a, T: EventHook> Continue<'a, T> {
    pub fn new(debugger: &'a Debugger<T>) -> Self {
        Self { dbg: debugger }
    }

    pub fn run(&self) -> command::Result<()> {
        Ok(self
            .dbg
            .continue_execution()
            .context("Failed to continue execution")?)
    }
}
