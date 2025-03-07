use crate::debugger::Debugger;
use crate::debugger::variable::dqe::Literal;
use crate::ui::command;

#[derive(Debug, Clone)]
pub struct Command {
    pub fn_name: String,
    pub args: Box<[Literal]>,
}

pub struct Handler<'a> {
    dbg: &'a mut Debugger,
}

impl<'a> Handler<'a> {
    pub fn new(debugger: &'a mut Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&mut self, cmd: Command) -> command::CommandResult<()> {
        self.dbg.call(&cmd.fn_name, &cmd.args)?;
        Ok(())
    }
}
