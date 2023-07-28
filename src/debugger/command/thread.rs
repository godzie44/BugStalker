use crate::debugger::debugee::tracee::Tracee;
use crate::debugger::{command, Debugger, ThreadSnapshot};

#[derive(Debug)]
pub enum Command {
    Dump,
    Current,
    Switch(u32),
}

pub struct Thread<'a> {
    dbg: &'a mut Debugger,
}

pub enum Result {
    List(Vec<ThreadSnapshot>),
    BroughtIntoFocus(Tracee),
}

impl<'a> Thread<'a> {
    pub fn new(debugger: &'a mut Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&mut self, cmd: Command) -> command::HandleResult<Result> {
        match cmd {
            Command::Dump => {
                let state = self.dbg.thread_state()?;
                Ok(Result::List(state))
            }
            Command::Current => {
                let state = self.dbg.thread_state()?;
                Ok(Result::List(
                    state.into_iter().filter(|t| t.in_focus).collect(),
                ))
            }
            Command::Switch(num) => {
                let in_focus_tracee = self.dbg.set_thread_into_focus(num)?;
                Ok(Result::BroughtIntoFocus(in_focus_tracee))
            }
        }
    }
}
