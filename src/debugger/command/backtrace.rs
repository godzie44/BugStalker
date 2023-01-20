use crate::debugger::{command, uw, Debugger};

pub struct Backtrace<'a> {
    dbg: &'a Debugger,
}

impl<'a> Backtrace<'a> {
    pub fn new(debugger: &'a Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn run(&self) -> command::Result<uw::Backtrace> {
        Ok(self
            .dbg
            .backtrace(self.dbg.debugee.threads_ctl.thread_in_focus())?)
    }
}
