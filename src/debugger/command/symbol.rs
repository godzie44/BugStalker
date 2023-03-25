use crate::debugger::debugee::dwarf;
use crate::debugger::{command, Debugger};

pub struct Symbol<'a> {
    dbg: &'a Debugger,
}

impl<'a> Symbol<'a> {
    pub fn new(debugger: &'a Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(self, symbol_name: &str) -> command::HandleResult<&'a dwarf::Symbol> {
        Ok(self.dbg.get_symbol(symbol_name)?)
    }
}
