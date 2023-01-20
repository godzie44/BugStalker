use crate::debugger::{command, Debugger};
use anyhow::Context;

pub struct Continue<'a> {
    dbg: &'a mut Debugger,
}

impl<'a> Continue<'a> {
    pub fn new(debugger: &'a mut Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn run(&mut self) -> command::Result<()> {
        Ok(self
            .dbg
            .continue_execution()
            .context("Failed to continue execution")?)
    }
}
