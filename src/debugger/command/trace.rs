use crate::debugger::{Debugger, ThreadDump};

pub struct Trace<'a> {
    dbg: &'a Debugger,
}

impl<'a> Trace<'a> {
    pub fn new(debugger: &'a Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn run(&self) -> anyhow::Result<Vec<ThreadDump>> {
        let mut dump = self.dbg.thread_state()?;
        dump.sort_unstable_by(|t1, t2| t1.thread.pid.cmp(&t2.thread.pid));
        Ok(dump)
    }
}
