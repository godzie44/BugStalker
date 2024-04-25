use crate::debugger::Debugger;
use crate::debugger::Error;

#[derive(Debug, Clone)]
pub enum WatchpointIdentity {
    Variable(String),
    Address(usize),
    Number(u32),
}

#[derive(Debug, Clone)]
pub enum Command {
    Add(WatchpointIdentity),
    Remove(WatchpointIdentity),
    Info,
}

pub struct Handler<'a> {
    #[allow(unused)]
    dbg: &'a mut Debugger,
}

pub enum ExecutionResult {
    New,
    Removed,
    Dump,
}

impl<'a> Handler<'a> {
    pub fn new(debugger: &'a mut Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&mut self, cmd: &Command) -> Result<ExecutionResult, Error> {
        match cmd {
            Command::Add(ident) => match ident {
                WatchpointIdentity::Variable(_) => {
                    unimplemented!()
                }
                WatchpointIdentity::Address(_) => {
                    unimplemented!()
                }
                WatchpointIdentity::Number(_) => {
                    unreachable!()
                }
            },
            Command::Remove(_) => {
                todo!()
            }
            Command::Info => {
                todo!()
            }
        };
    }
}
