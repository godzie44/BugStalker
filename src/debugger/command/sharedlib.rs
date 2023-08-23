use crate::debugger::debugee::RegionInfo;
use crate::debugger::Debugger;

pub struct SharedLib<'a> {
    dbg: &'a Debugger,
}

impl<'a> SharedLib<'a> {
    pub fn new(debugger: &'a Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&self) -> Vec<RegionInfo> {
        self.dbg.shared_libs()
    }
}
