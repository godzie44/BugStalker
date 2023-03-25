use crate::debugger::variable::select::Expression;
use crate::debugger::variable::VariableIR;
use crate::debugger::{command, Debugger};

pub struct Arguments<'a> {
    dbg: &'a Debugger,
}

impl<'a> Arguments<'a> {
    pub fn new(debugger: &'a Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&self, select_expression: Expression) -> command::HandleResult<Vec<VariableIR>> {
        Ok(self.dbg.read_argument(select_expression)?)
    }
}
