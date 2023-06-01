use crate::debugger::{command, Debugger};
use anyhow::Context;

pub struct Run<'a> {
    dbg: &'a mut Debugger,
}

impl<'a> Run<'a> {
    pub fn new(debugger: &'a mut Debugger) -> Self {
        Self { dbg: debugger }
    }

    /// Runs a debugee program.
    pub fn start(&mut self) -> command::HandleResult<()> {
        Ok(self.dbg.start_debugee().context("start fail")?)
    }

    /// Restart debugee process with saving all user defined breakpoints.
    pub fn restart(&mut self) -> command::HandleResult<()> {
        Ok(self.dbg.restart_debugee().context("restart fail")?)
    }
}
