use crate::debugger::Debugger;
use crate::debugger::RegionInfo;

pub struct Handler<'a> {
    dbg: &'a Debugger,
}

impl<'a> Handler<'a> {
    pub fn new(debugger: &'a Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&self) -> Vec<RegionInfo> {
        self.dbg.shared_libs()
    }
}
