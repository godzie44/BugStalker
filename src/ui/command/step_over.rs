use crate::debugger::Debugger;
use crate::ui::command;

/// Step program, proceeding through subroutine calls.
/// Unlike "step", if the current source line calls a subroutine,
/// this command does not enter the subroutine, but instead steps over
/// the call, in effect treating it as a single source line.
pub struct Handler<'a> {
    dbg: &'a mut Debugger,
}

impl<'a> Handler<'a> {
    pub fn new(debugger: &'a mut Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&mut self) -> command::CommandResult<()> {
        Ok(self.dbg.step_over()?)
    }
}
