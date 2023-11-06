use crate::debugger::address::Address;
use crate::debugger::breakpoint::BreakpointView;
use crate::debugger::error::Error;
use crate::debugger::Debugger;

#[derive(Debug, Clone)]
pub enum BreakpointRequest {
    Address(usize),
    Line(String, u64),
    Function(String),
    Number(u32),
}

#[derive(Debug)]
pub enum Command {
    Add(BreakpointRequest),
    Remove(BreakpointRequest),
    Info,
    AddDeferred(BreakpointRequest),
}

impl Command {
    /// Return underline breakpoint request (if exists).
    pub fn breakpoint(&self) -> Option<BreakpointRequest> {
        match self {
            Command::Add(b) => Some(b.clone()),
            Command::Remove(b) => Some(b.clone()),
            Command::Info => None,
            Command::AddDeferred(b) => Some(b.clone()),
        }
    }
}

pub struct Break<'a> {
    dbg: &'a mut Debugger,
}

pub enum HandlingResult<'a> {
    New(Vec<BreakpointView<'a>>),
    Removed(Vec<BreakpointView<'a>>),
    Dump(Vec<BreakpointView<'a>>),
    AddDeferred,
}

impl<'a> Break<'a> {
    pub fn new(debugger: &'a mut Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&mut self, cmd: &Command) -> Result<HandlingResult, Error> {
        let result = match cmd {
            Command::Add(brkpt) => {
                let res = match brkpt {
                    BreakpointRequest::Address(addr) => {
                        vec![self.dbg.set_breakpoint_at_addr((*addr).into())?]
                    }
                    BreakpointRequest::Line(file, line) => {
                        self.dbg.set_breakpoint_at_line(file, *line)?
                    }
                    BreakpointRequest::Function(func_name) => {
                        self.dbg.set_breakpoint_at_fn(func_name)?
                    }
                    BreakpointRequest::Number(_) => {
                        unimplemented!()
                    }
                };
                HandlingResult::New(res)
            }
            Command::Remove(brkpt) => {
                let res = match brkpt {
                    BreakpointRequest::Address(addr) => self
                        .dbg
                        .remove_breakpoint(Address::Relocated((*addr).into()))?
                        .map(|brkpt| vec![brkpt])
                        .unwrap_or_default(),
                    BreakpointRequest::Line(file, line) => {
                        self.dbg.remove_breakpoint_at_line(file, *line)?
                    }
                    BreakpointRequest::Function(func_name) => {
                        self.dbg.remove_breakpoint_at_fn(func_name)?
                    }
                    BreakpointRequest::Number(number) => self
                        .dbg
                        .remove_breakpoint_by_number(*number)?
                        .map(|brkpt| vec![brkpt])
                        .unwrap_or_default(),
                };
                HandlingResult::Removed(res)
            }
            Command::Info => HandlingResult::Dump(self.dbg.breakpoints_snapshot()),
            Command::AddDeferred(brkpt) => {
                match brkpt {
                    BreakpointRequest::Address(addr) => {
                        self.dbg.add_deferred_at_addr((*addr).into())
                    }
                    BreakpointRequest::Line(file, line) => {
                        self.dbg.add_deferred_at_line(file, *line)
                    }
                    BreakpointRequest::Function(function) => {
                        self.dbg.add_deferred_at_function(function)
                    }
                    BreakpointRequest::Number(_) => {
                        unimplemented!()
                    }
                };
                HandlingResult::AddDeferred
            }
        };
        Ok(result)
    }
}
