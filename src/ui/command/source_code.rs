use crate::debugger::{Debugger, Error, FunctionAssembly, FunctionRange};

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Range(u64),
    Function,
    Asm,
}

pub struct DisAsmHandler<'a> {
    dbg: &'a Debugger,
}

impl<'a> DisAsmHandler<'a> {
    pub fn new(debugger: &'a Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&self) -> Result<FunctionAssembly, Error> {
        self.dbg.disasm()
    }
}

pub struct FunctionLineRangeHandler<'a> {
    dbg: &'a Debugger,
}

impl<'a> FunctionLineRangeHandler<'a> {
    pub fn new(debugger: &'a Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&self) -> Result<FunctionRange<'_>, Error> {
        self.dbg.current_function_range()
    }
}
