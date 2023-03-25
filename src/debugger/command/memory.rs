use crate::debugger::{command, Debugger};
use anyhow::anyhow;
use nix::libc::uintptr_t;
use std::mem;

#[derive(Debug)]
pub enum Command {
    Read(usize),
    Write(usize, uintptr_t),
}

pub struct Memory<'a> {
    dbg: &'a Debugger,
}

impl<'a> Memory<'a> {
    pub fn new(debugger: &'a Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(&self, cmd: Command) -> command::HandleResult<uintptr_t> {
        let result = match &cmd {
            Command::Read(addr) => {
                let bytes = self
                    .dbg
                    .read_memory(*addr, mem::size_of::<usize>())
                    .map_err(anyhow::Error::from)?;
                uintptr_t::from_ne_bytes(bytes.try_into().map_err(|e| anyhow!("{e:?}"))?)
            }
            Command::Write(addr, ptr) => {
                self.dbg
                    .write_memory(*addr, *ptr)
                    .map_err(anyhow::Error::from)?;
                *ptr
            }
        };

        Ok(result)
    }
}
