use crate::debugger::breakpoint::BreakpointView;
use crate::debugger::Debugger;

pub use crate::debugger::breakpoint::SetBreakpointError;

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
    AddDeferred(Breakpoint),
}

impl Command {
    /// Return underline breakpoint request (if exists).
    pub fn breakpoint(&self) -> Option<Breakpoint> {
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

#[derive(Debug, thiserror::Error)]
pub enum BreakpointError {
    #[error(transparent)]
    SetError(#[from] SetBreakpointError),
    #[error(transparent)]
    OtherError(#[from] anyhow::Error),
}

impl<'a> Break<'a> {
    pub fn new(debugger: &'a mut Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&mut self, cmd: &Command) -> Result<HandlingResult, BreakpointError> {
        let result = match cmd {
            Command::Add(brkpt) => {
                let res = match brkpt {
                    Breakpoint::Address(addr) => {
                        vec![self.dbg.set_breakpoint_at_addr((*addr).into())?]
                    }
                    Breakpoint::Line(file, line) => self.dbg.set_breakpoint_at_line(file, *line)?,
                    Breakpoint::Function(func_name) => self.dbg.set_breakpoint_at_fn(&func_name)?,
                };
                HandlingResult::New(res)
            }
            Command::Remove(brkpt) => {
                let res = match brkpt {
                    Breakpoint::Address(addr) => self
                        .dbg
                        .remove_breakpoint_at_addr((*addr).into())?
                        .map(|brkpt| vec![brkpt])
                        .unwrap_or_default(),
                    Breakpoint::Line(file, line) => {
                        self.dbg.remove_breakpoint_at_line(file, *line)?
                    }
                    Breakpoint::Function(func_name) => {
                        self.dbg.remove_breakpoint_at_fn(func_name)?
                    }
                };
                HandlingResult::Removed(res)
            }
            Command::Info => HandlingResult::Dump(self.dbg.breakpoints_snapshot()),
            Command::AddDeferred(brkpt) => {
                match brkpt {
                    Breakpoint::Address(addr) => self.dbg.add_deferred_at_addr((*addr).into()),
                    Breakpoint::Line(file, line) => self.dbg.add_deferred_at_line(file, *line),
                    Breakpoint::Function(function) => self.dbg.add_deferred_at_function(function),
                };
                HandlingResult::AddDeferred
            }
        };
        Ok(result)
    }
}
