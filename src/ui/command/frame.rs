use crate::debugger::{Debugger, FrameInfo};
use crate::ui::command;

#[derive(Debug, Clone)]
pub enum Command {
    Info,
    Switch(u32),
}

pub enum ExecutionResult {
    FrameInfo(FrameInfo),
    BroughtIntoFocus(u32),
}

pub struct Handler<'a> {
    dbg: &'a mut Debugger,
}

impl<'a> Handler<'a> {
    pub fn new(debugger: &'a mut Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&mut self, cmd: Command) -> command::CommandResult<ExecutionResult> {
        match cmd {
            Command::Info => {
                let info = self.dbg.frame_info()?;
                Ok(ExecutionResult::FrameInfo(info))
            }
            Command::Switch(num) => {
                self.dbg.set_frame_into_focus(num)?;
                Ok(ExecutionResult::BroughtIntoFocus(num))
            }
        }
    }
}
