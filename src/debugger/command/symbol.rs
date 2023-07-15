use crate::debugger::debugee::dwarf;
use crate::debugger::{command, Debugger};

pub struct Symbol<'a> {
    dbg: &'a Debugger,
}

impl<'a> Symbol<'a> {
    pub fn new(debugger: &'a Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(self, regex: &str) -> command::HandleResult<Vec<&'a dwarf::Symbol>> {
        let mut symbols = self.dbg.get_symbols(regex)?;
        symbols.sort_by(|s1, s2| s1.name.cmp(&s2.name));
        Ok(symbols)
    }
}
