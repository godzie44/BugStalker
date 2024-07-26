use crate::debugger::Debugger;

pub struct TokioAnalyzeContext<'a> {
    debugger: &'a mut Debugger,
}

impl<'a> TokioAnalyzeContext<'a> {
    pub fn new(debugger: &'a mut Debugger) -> Self {
        Self { debugger }
    }

    pub fn debugger_mut(&mut self) -> &mut Debugger {
        self.debugger
    }

    pub fn debugger(&self) -> &Debugger {
        self.debugger
    }
}
