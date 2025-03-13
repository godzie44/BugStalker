use crate::debugger::Debugger;
use crate::debugger::variable::dqe::Dqe;
use crate::debugger::variable::execute::QueryResult;
use crate::ui::command;

#[derive(Debug, Clone, PartialEq)]
pub enum RenderMode {
    Builtin,
    Debug,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Variable { mode: RenderMode, dqe: Dqe },
    Argument { mode: RenderMode, dqe: Dqe },
}

pub struct Handler<'a> {
    dbg: &'a Debugger,
}

impl<'a> Handler<'a> {
    pub fn new(debugger: &'a Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(self, cmd: Command) -> command::CommandResult<Vec<QueryResult<'a>>> {
        Ok(match cmd {
            Command::Variable { dqe, .. } => self.dbg.read_variable(dqe)?,
            Command::Argument { dqe, .. } => self.dbg.read_argument(dqe)?,
        })
    }
}
