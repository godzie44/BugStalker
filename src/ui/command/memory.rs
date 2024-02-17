use crate::debugger::Debugger;
use crate::debugger::Error;
use crate::ui::command;
use nix::libc::uintptr_t;
use std::mem;

#[derive(Debug, Clone)]
pub enum Command {
    Read(usize),
    Write(usize, uintptr_t),
}

pub struct Handler<'a> {
    dbg: &'a Debugger,
}

impl<'a> Handler<'a> {
    pub fn new(debugger: &'a Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&self, cmd: Command) -> command::CommandResult<uintptr_t> {
        let result = match &cmd {
            Command::Read(addr) => {
                let bytes = self.dbg.read_memory(*addr, mem::size_of::<usize>())?;
                uintptr_t::from_ne_bytes(bytes.try_into().map_err(|data: Vec<u8>| {
                    Error::TypeBinaryRepr("uintptr_t", data.into_boxed_slice())
                })?)
            }
            Command::Write(addr, ptr) => {
                self.dbg.write_memory(*addr, *ptr)?;
                *ptr
            }
        };

        Ok(result)
    }
}
