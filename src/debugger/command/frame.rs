use crate::debugger::{command, Debugger, FrameInfo};

#[derive(Debug)]
pub enum Command {
    Info,
    Switch(u32),
}

pub enum Result {
    FrameInfo(FrameInfo),
    BroughtIntoFocus(u32),
}

pub struct Frame<'a> {
    dbg: &'a mut Debugger,
}

impl<'a> Frame<'a> {
    pub fn new(debugger: &'a mut Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&mut self, cmd: Command) -> command::HandleResult<Result> {
        match cmd {
            Command::Info => {
                let info = self.dbg.frame_info()?;
                Ok(Result::FrameInfo(info))
            }
            Command::Switch(num) => {
                self.dbg.set_frame_into_focus(num)?;
                Ok(Result::BroughtIntoFocus(num))
            }
        }
    }
}
