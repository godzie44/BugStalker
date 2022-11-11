use crate::debugger::{command, Debugger, EventHook, FrameInfo};

pub struct Frame<'a, T: EventHook> {
    dbg: &'a Debugger<'a, T>,
}

impl<'a, T: EventHook> Frame<'a, T> {
    pub fn new(debugger: &'a Debugger<'a, T>) -> Self {
        Self { dbg: debugger }
    }

    pub fn run(&self) -> command::Result<FrameInfo> {
        Ok(self.dbg.frame_info()?)
    }
}
