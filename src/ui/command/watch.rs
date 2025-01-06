use crate::debugger::address::RelocatedAddress;
use crate::debugger::register::debug::{BreakCondition, BreakSize};
use crate::debugger::variable::dqe::Dqe;
use crate::debugger::Debugger;
use crate::debugger::Error;
use crate::debugger::WatchpointView;

#[derive(Debug, Clone)]
pub enum WatchpointIdentity {
    DQE(String, Dqe),
    Address(usize, u8),
    Number(u32),
}

#[derive(Debug, Clone)]
pub enum Command {
    Add(WatchpointIdentity, BreakCondition),
    Remove(WatchpointIdentity),
    Info,
}

pub struct Handler<'a> {
    dbg: &'a mut Debugger,
}

pub enum ExecutionResult<'a> {
    New(WatchpointView<'a>),
    Removed(Option<WatchpointView<'a>>),
    Dump(Vec<WatchpointView<'a>>),
}

impl<'a> Handler<'a> {
    pub fn new(debugger: &'a mut Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&mut self, cmd: Command) -> Result<ExecutionResult, Error> {
        match cmd {
            Command::Add(ident, cond) => {
                let new = match ident {
                    WatchpointIdentity::DQE(expr_string, dqe) => {
                        self.dbg.set_watchpoint_on_expr(&expr_string, dqe, cond)
                    }
                    WatchpointIdentity::Address(addr, size) => self.dbg.set_watchpoint_on_memory(
                        RelocatedAddress::from(addr),
                        BreakSize::try_from(size).expect("infallible (checked by parser)"),
                        cond,
                        false,
                    ),
                    WatchpointIdentity::Number(_) => {
                        unreachable!()
                    }
                }?;
                Ok(ExecutionResult::New(new))
            }
            Command::Remove(ident) => {
                let rem = match ident {
                    WatchpointIdentity::DQE(_, dqe) => self.dbg.remove_watchpoint_by_expr(dqe),
                    WatchpointIdentity::Address(addr, _) => self
                        .dbg
                        .remove_watchpoint_by_addr(RelocatedAddress::from(addr)),
                    WatchpointIdentity::Number(num) => self.dbg.remove_watchpoint_by_number(num),
                }?;
                Ok(ExecutionResult::Removed(rem))
            }
            Command::Info => {
                let list = self.dbg.watchpoint_list();
                Ok(ExecutionResult::Dump(list))
            }
        }
    }
}
