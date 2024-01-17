use crate::debugger::Debugger;
use crate::ui::command;

pub enum Command {
    Start,
    DryStart,
    Restart,
}

pub struct Handler<'a> {
    dbg: &'a mut Debugger,
}

impl<'a> Handler<'a> {
    pub fn new(debugger: &'a mut Debugger) -> Self {
        Self { dbg: debugger }
    }

    /// Run or restart (with saving all user defined breakpoints) a debugee program.
    /// Return when debugee stopped or ends.
    pub fn handle(&mut self, cmd: Command) -> command::CommandResult<()> {
        match cmd {
            Command::Start => Ok(self.dbg.start_debugee()?),
            Command::Restart => {
                self.dbg.start_debugee_force()?;
                Ok(())
            }
            Command::DryStart => Ok(self.dbg.dry_start_debugee()?),
        }
    }
}
