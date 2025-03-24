use log::warn;

use crate::debugger::call::fmt::DebugFormattable;
use crate::debugger::variable::dqe::Dqe;
use crate::debugger::variable::execute::QueryResult;
use crate::debugger::{self, Debugger};
use crate::ui::command;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RenderMode {
    Builtin,
    Debug,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Variable { mode: RenderMode, dqe: Dqe },
    Argument { mode: RenderMode, dqe: Dqe },
}

impl Command {
    fn render_mode(&self) -> RenderMode {
        match self {
            Command::Variable { mode, .. } => *mode,
            Command::Argument { mode, .. } => *mode,
        }
    }
}

pub enum ReadVariableResult<'a> {
    PreRender(QueryResult<'a>, String),
    Raw(QueryResult<'a>),
}

pub struct Handler<'a> {
    dbg: &'a Debugger,
}

impl<'a> Handler<'a> {
    pub fn new(debugger: &'a Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(self, cmd: Command) -> command::CommandResult<Vec<ReadVariableResult<'a>>> {
        let render_mode = cmd.render_mode();
        let read_result = match cmd {
            Command::Variable { dqe, .. } => self.dbg.read_variable(dqe)?,
            Command::Argument { dqe, .. } => self.dbg.read_argument(dqe)?,
        };

        Ok(self.prepare_results(read_result, render_mode))
    }

    fn prepare_results(
        &self,
        read_result: Vec<QueryResult<'a>>,
        mode: RenderMode,
    ) -> Vec<ReadVariableResult<'a>> {
        let rr_iter = read_result.into_iter();
        if mode == RenderMode::Builtin {
            return rr_iter.map(ReadVariableResult::Raw).collect();
        }

        rr_iter
            .map(|qr| {
                // Call debug trait only for some types
                if qr.value().formattable()  {
                    match debugger::call::fmt::call_debug_fmt(self.dbg, &qr) {
                        Ok(s) => ReadVariableResult::PreRender(qr, s),
                        Err(e) => {
                            warn!(target: "debugger", "error {} while render variable {} using Debug trait, fallback to a builtin render", e, qr.identity());
                            ReadVariableResult::Raw(qr)
                        }
                    }
                } else {
                    ReadVariableResult::Raw(qr)
                }
            })
            .collect()
    }
}
