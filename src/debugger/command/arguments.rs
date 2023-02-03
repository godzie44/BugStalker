use crate::debugger::variable::VariableIR;
use crate::debugger::{command, Debugger};

pub struct Arguments<'a> {
    dbg: &'a Debugger,
}

impl<'a> Arguments<'a> {
    pub fn new(debugger: &'a Debugger) -> command::Result<Self> {
        Ok(Self { dbg: debugger })
    }

    pub fn run(&self) -> command::Result<Vec<VariableIR>> {
        Ok(self.dbg.read_arguments()?)
    }
}
