use crate::debugger::{command, Debugger, EventHook};
use anyhow::Context;

pub struct Continue<'a, T: EventHook> {
    dbg: &'a Debugger<'a, T>,
}

impl<'a, T: EventHook> Continue<'a, T> {
    pub fn new(debugger: &'a Debugger<'a, T>) -> Self {
        Self { dbg: debugger }
    }

    pub fn run(&self) -> command::Result<()> {
        Ok(self
            .dbg
            .continue_execution()
            .context("Failed to continue execution")?)
    }
}
