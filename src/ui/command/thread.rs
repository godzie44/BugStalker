use crate::debugger::Tracee;
use crate::debugger::{Debugger, ThreadSnapshot};
use crate::ui::command;

#[derive(Debug, Clone)]
pub enum Command {
    Info,
    Current,
    Switch(u32),
}

pub struct Handler<'a> {
    dbg: &'a mut Debugger,
}

pub enum ExecutionResult {
    List(Vec<ThreadSnapshot>),
    BroughtIntoFocus(Tracee),
}

impl<'a> Handler<'a> {
    pub fn new(debugger: &'a mut Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&mut self, cmd: Command) -> command::CommandResult<ExecutionResult> {
        match cmd {
            Command::Info => {
                let state = self.dbg.thread_state()?;
                Ok(ExecutionResult::List(state))
            }
            Command::Current => {
                let state = self.dbg.thread_state()?;
                Ok(ExecutionResult::List(
                    state.into_iter().filter(|t| t.in_focus).collect(),
                ))
            }
            Command::Switch(num) => {
                let in_focus_tracee = self.dbg.set_thread_into_focus(num)?;
                Ok(ExecutionResult::BroughtIntoFocus(in_focus_tracee))
            }
        }
    }
}
