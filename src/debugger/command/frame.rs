use crate::debugger::{command, Debugger, EventHook, FrameInfo};

pub struct Frame<'a, T: EventHook> {
    dbg: &'a Debugger<T>,
}

impl<'a, T: EventHook> Frame<'a, T> {
    pub fn new(debugger: &'a Debugger<T>) -> Self {
        Self { dbg: debugger }
    }

    pub fn run(&self) -> command::Result<FrameInfo> {
        Ok(self.dbg.frame_info()?)
    }
}
