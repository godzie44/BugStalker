use crate::debugger::address::PCValue;
use crate::debugger::{command, Debugger};

#[derive(Debug, Clone)]
pub enum Breakpoint {
    Address(usize),
    Line(String, u64),
    Function(String),
}

pub struct Break<'a> {
    dbg: &'a mut Debugger,
}

impl<'a> Break<'a> {
    pub fn new(debugger: &'a mut Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&mut self, cmd: Breakpoint) -> command::HandleResult<()> {
        match cmd {
            Breakpoint::Address(addr) => {
                Ok(self.dbg.set_breakpoint(PCValue::Relocated((addr).into()))?)
            }
            Breakpoint::Line(file, line) => Ok(self.dbg.set_breakpoint_at_line(&file, line)?),
            Breakpoint::Function(func_name) => Ok(self.dbg.set_breakpoint_at_fn(&func_name)?),
        }
    }
}
