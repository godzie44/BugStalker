use crate::debugger::{Debugger, Error, FunctionAssembly};

pub struct Handler<'a> {
    dbg: &'a Debugger,
}

impl<'a> Handler<'a> {
    pub fn new(debugger: &'a Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&self) -> Result<FunctionAssembly, Error> {
        self.dbg.disasm()
    }
}
