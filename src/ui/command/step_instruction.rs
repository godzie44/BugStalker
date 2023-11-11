use crate::debugger::Debugger;
use crate::ui::command;

/// Step on next instruction
pub struct Handler<'a> {
    dbg: &'a mut Debugger,
}

impl<'a> Handler<'a> {
    pub fn new(debugger: &'a mut Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&mut self) -> command::CommandResult<()> {
        Ok(self.dbg.stepi()?)
    }
}
