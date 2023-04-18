use crate::debugger::{command, Debugger, ThreadSnapshot};

#[derive(Debug)]
pub enum Command {
    CurrentThread,
    All,
}

pub struct Backtrace<'a> {
    dbg: &'a Debugger,
}

impl<'a> Backtrace<'a> {
    pub fn new(debugger: &'a Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&self, cmd: Command) -> command::HandleResult<Vec<ThreadSnapshot>> {
        let mut snap = self.dbg.thread_state()?;

        match cmd {
            Command::CurrentThread => {
                Ok(snap.into_iter().filter(|thread| thread.in_focus).collect())
            }
            Command::All => {
                snap.sort_unstable_by(|t1, t2| t1.thread.pid.cmp(&t2.thread.pid));
                Ok(snap)
            }
        }
    }
}
