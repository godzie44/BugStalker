use crate::debugger::{command, Debugger};

pub struct Continue<'a> {
    dbg: &'a mut Debugger,
}

impl<'a> Continue<'a> {
    pub fn new(debugger: &'a mut Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&mut self) -> command::HandleResult<()> {
        self.dbg.continue_debugee()?;
        Ok(())
    }
}
