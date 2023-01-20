use crate::debugger::{command, Debugger, FrameInfo};

pub struct Frame<'a> {
    dbg: &'a Debugger,
}

impl<'a> Frame<'a> {
    pub fn new(debugger: &'a Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn run(&self) -> command::Result<FrameInfo> {
        Ok(self
            .dbg
            .frame_info(self.dbg.debugee.threads_ctl.thread_in_focus())?)
    }
}
