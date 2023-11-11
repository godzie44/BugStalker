use crate::debugger::address::Address;
use crate::debugger::BreakpointView;
use crate::debugger::Debugger;
use crate::debugger::Error;

#[derive(Debug, Clone)]
pub enum BreakpointIdentity {
    Address(usize),
    Line(String, u64),
    Function(String),
    Number(u32),
}

#[derive(Debug)]
pub enum Command {
    Add(BreakpointIdentity),
    Remove(BreakpointIdentity),
    Info,
    AddDeferred(BreakpointIdentity),
}

impl Command {
    /// Return underline breakpoint identity (if command not an `info`).
    pub fn identity(&self) -> Option<BreakpointIdentity> {
        match self {
            Command::Add(b) => Some(b.clone()),
            Command::Remove(b) => Some(b.clone()),
            Command::Info => None,
            Command::AddDeferred(b) => Some(b.clone()),
        }
    }
}

pub struct Handler<'a> {
    dbg: &'a mut Debugger,
}

pub enum ExecutionResult<'a> {
    New(Vec<BreakpointView<'a>>),
    Removed(Vec<BreakpointView<'a>>),
    Dump(Vec<BreakpointView<'a>>),
    AddDeferred,
}

impl<'a> Handler<'a> {
    pub fn new(debugger: &'a mut Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&mut self, cmd: &Command) -> Result<ExecutionResult, Error> {
        let result = match cmd {
            Command::Add(brkpt) => {
                let res = match brkpt {
                    BreakpointIdentity::Address(addr) => {
                        vec![self.dbg.set_breakpoint_at_addr((*addr).into())?]
                    }
                    BreakpointIdentity::Line(file, line) => {
                        self.dbg.set_breakpoint_at_line(file, *line)?
                    }
                    BreakpointIdentity::Function(func_name) => {
                        self.dbg.set_breakpoint_at_fn(func_name)?
                    }
                    BreakpointIdentity::Number(_) => {
                        unimplemented!()
                    }
                };
                ExecutionResult::New(res)
            }
            Command::Remove(brkpt) => {
                let res = match brkpt {
                    BreakpointIdentity::Address(addr) => self
                        .dbg
                        .remove_breakpoint(Address::Relocated((*addr).into()))?
                        .map(|brkpt| vec![brkpt])
                        .unwrap_or_default(),
                    BreakpointIdentity::Line(file, line) => {
                        self.dbg.remove_breakpoint_at_line(file, *line)?
                    }
                    BreakpointIdentity::Function(func_name) => {
                        self.dbg.remove_breakpoint_at_fn(func_name)?
                    }
                    BreakpointIdentity::Number(number) => self
                        .dbg
                        .remove_breakpoint_by_number(*number)?
                        .map(|brkpt| vec![brkpt])
                        .unwrap_or_default(),
                };
                ExecutionResult::Removed(res)
            }
            Command::Info => ExecutionResult::Dump(self.dbg.breakpoints_snapshot()),
            Command::AddDeferred(brkpt) => {
                match brkpt {
                    BreakpointIdentity::Address(addr) => {
                        self.dbg.add_deferred_at_addr((*addr).into())
                    }
                    BreakpointIdentity::Line(file, line) => {
                        self.dbg.add_deferred_at_line(file, *line)
                    }
                    BreakpointIdentity::Function(function) => {
                        self.dbg.add_deferred_at_function(function)
                    }
                    BreakpointIdentity::Number(_) => {
                        unimplemented!()
                    }
                };
                ExecutionResult::AddDeferred
            }
        };
        Ok(result)
    }
}
