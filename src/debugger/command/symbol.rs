use crate::debugger::debugee::dwarf;
use crate::debugger::{command, Debugger};

pub struct Symbol<'a> {
    dbg: &'a Debugger,
    name: String,
}

impl<'a> Symbol<'a> {
    pub fn new<'s>(debugger: &'a Debugger, args: Vec<&'s str>) -> command::Result<Self> {
        command::helper::check_args_count(&args, 2)?;
        Ok(Self {
            dbg: debugger,
            name: args[1].into(),
        })
    }

    pub fn run(&self) -> command::Result<&dwarf::Symbol> {
        Ok(self.dbg.get_symbol(&self.name)?)
    }
}
