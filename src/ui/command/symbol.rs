use crate::debugger;
use crate::debugger::Debugger;
use crate::ui::command;

pub struct Handler<'a> {
    dbg: &'a Debugger,
}

impl<'a> Handler<'a> {
    pub fn new(debugger: &'a Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(self, regex: &str) -> command::CommandResult<Vec<debugger::Symbol<'a>>> {
        let mut symbols = self.dbg.get_symbols(regex)?;
        symbols.sort_by(|s1, s2| s1.name.cmp(s2.name));
        Ok(symbols)
    }
}
