use crate::debugger::{command, Debugger, EventHook};

/// Step program, proceeding through subroutine calls.
/// Unlike "step", if the current source line calls a subroutine,
/// this command does not enter the subroutine, but instead steps over
/// the call, in effect treating it as a single source line.
pub struct StepOver<'a, T: EventHook> {
    dbg: &'a Debugger<T>,
}

impl<'a, T: EventHook> StepOver<'a, T> {
    pub fn new(debugger: &'a Debugger<T>) -> Self {
        Self { dbg: debugger }
    }

    pub fn run(&self) -> command::Result<()> {
        Ok(self.dbg.step_over()?)
    }
}
