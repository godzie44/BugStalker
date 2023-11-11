use crate::debugger::variable::select::Expression;
use crate::debugger::variable::VariableIR;
use crate::debugger::Debugger;
use crate::ui::command;

pub struct Handler<'a> {
    dbg: &'a Debugger,
}

impl<'a> Handler<'a> {
    pub fn new(debugger: &'a Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(self, select_expression: Expression) -> command::CommandResult<Vec<VariableIR>> {
        Ok(self.dbg.read_variable(select_expression)?)
    }
}
