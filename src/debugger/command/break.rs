use crate::debugger::address::Address;
use crate::debugger::{command, Debugger};

#[derive(Debug, Clone)]
pub enum Breakpoint {
    Address(usize),
    Line(String, u64),
    Function(String),
}

#[derive(Debug)]
pub enum Command {
    Add(Breakpoint),
    Remove(Breakpoint),
}

pub struct Break<'a> {
    dbg: &'a mut Debugger,
}

impl<'a> Break<'a> {
    pub fn new(debugger: &'a mut Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&mut self, cmd: Command) -> command::HandleResult<()> {
        let result = match cmd {
            Command::Add(brkpt) => match brkpt {
                Breakpoint::Address(addr) => {
                    self.dbg.set_breakpoint(Address::Relocated((addr).into()))
                }
                Breakpoint::Line(file, line) => self.dbg.set_breakpoint_at_line(&file, line),
                Breakpoint::Function(func_name) => self.dbg.set_breakpoint_at_fn(&func_name),
            },
            Command::Remove(brkpt) => match brkpt {
                Breakpoint::Address(addr) => self
                    .dbg
                    .remove_breakpoint(Address::Relocated((addr).into())),
                Breakpoint::Line(file, line) => self.dbg.remove_breakpoint_at_line(&file, line),
                Breakpoint::Function(func_name) => self.dbg.remove_breakpoint_at_fn(&func_name),
            },
        };

        Ok(result?)
    }
}
