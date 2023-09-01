use crate::debugger::breakpoint::BreakpointView;
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
    Info,
}

pub struct Break<'a> {
    dbg: &'a mut Debugger,
}

pub enum HandlingResult<'a> {
    New(BreakpointView<'a>),
    Removed(Option<BreakpointView<'a>>),
    Dump(Vec<BreakpointView<'a>>),
}

impl<'a> Break<'a> {
    pub fn new(debugger: &'a mut Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&mut self, cmd: Command) -> command::HandleResult<HandlingResult> {
        let result = match cmd {
            Command::Add(brkpt) => {
                let res = match brkpt {
                    Breakpoint::Address(addr) => self.dbg.set_breakpoint_at_addr(addr.into()),
                    Breakpoint::Line(file, line) => self.dbg.set_breakpoint_at_line(&file, line),
                    Breakpoint::Function(func_name) => self.dbg.set_breakpoint_at_fn(&func_name),
                };
                HandlingResult::New(res?)
            }
            Command::Remove(brkpt) => {
                let res = match brkpt {
                    Breakpoint::Address(addr) => self.dbg.remove_breakpoint_at_addr(addr.into()),
                    Breakpoint::Line(file, line) => self.dbg.remove_breakpoint_at_line(&file, line),
                    Breakpoint::Function(func_name) => self.dbg.remove_breakpoint_at_fn(&func_name),
                };
                HandlingResult::Removed(res?)
            }
            Command::Info => HandlingResult::Dump(self.dbg.breakpoints_snapshot()),
        };
        Ok(result)
    }
}
