use crate::debugger::{command, Debugger};
use anyhow::Context;

pub struct Run<'a> {
    dbg: &'a mut Debugger,
}

impl<'a> Run<'a> {
    pub fn new(debugger: &'a mut Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&mut self) -> command::HandleResult<()> {
        Ok(self.dbg.run_debugee().context("run fail")?)
    }
}
