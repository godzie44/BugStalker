use crate::debugger::Debugger;
use crate::debugger::variable::dqe::Dqe;
use crate::debugger::variable::execute::QueryResult;
use crate::ui::command;

pub struct Handler<'a> {
    dbg: &'a Debugger,
}

impl<'a> Handler<'a> {
    pub fn new(debugger: &'a Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&self, select_expression: Dqe) -> command::CommandResult<Vec<QueryResult>> {
        Ok(self.dbg.read_argument(select_expression)?)
    }
}
