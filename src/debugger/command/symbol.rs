use crate::debugger::{command, dwarf, Debugger, EventHook};

pub struct Symbol<'a, T: EventHook> {
    dbg: &'a Debugger<'a, T>,
    name: String,
}

impl<'a, T: EventHook> Symbol<'a, T> {
    pub fn new<'s>(debugger: &'a Debugger<'a, T>, args: Vec<&'s str>) -> command::Result<Self> {
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
