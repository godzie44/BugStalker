use crate::debugger::{command, Debugger, FrameInfo};

pub struct Frame<'a> {
    dbg: &'a Debugger,
}

impl<'a> Frame<'a> {
    pub fn new(debugger: &'a Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&self) -> command::HandleResult<FrameInfo> {
        Ok(self.dbg.frame_info(self.dbg.debugee.thread_in_focus())?)
    }
}
