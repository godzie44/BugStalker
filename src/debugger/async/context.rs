use crate::debugger::Debugger;

use super::tokio::TokioVersion;

pub struct TokioAnalyzeContext<'a> {
    debugger: &'a mut Debugger,
    _tokio_version: TokioVersion,
}

impl<'a> TokioAnalyzeContext<'a> {
    pub fn new(debugger: &'a mut Debugger, tokio_version: TokioVersion) -> Self {
        Self {
            debugger,
            _tokio_version: tokio_version,
        }
    }

    pub fn debugger_mut(&mut self) -> &mut Debugger {
        self.debugger
    }

    pub fn debugger(&self) -> &Debugger {
        self.debugger
    }
}
