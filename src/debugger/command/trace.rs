use crate::debugger::{Debugger, EventHook, ThreadDump};

pub struct Trace<'a, T: EventHook> {
    dbg: &'a Debugger<T>,
}

impl<'a, T: EventHook> Trace<'a, T> {
    pub fn new(debugger: &'a Debugger<T>) -> Self {
        Self { dbg: debugger }
    }

    pub fn run(&self) -> Vec<ThreadDump<'a>> {
        let mut dump = self.dbg.thread_state();
        dump.sort_unstable_by(|t1, t2| t1.thread.num.cmp(&t2.thread.num));
        dump
    }
}
