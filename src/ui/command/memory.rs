use crate::debugger::Debugger;
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

    pub fn handle(&self, cmd: Command) -> command::CommandResult<Vec<u8>> {
        let result = match &cmd {
            Command::Read(addr) => self.dbg.read_memory(*addr, mem::size_of::<usize>())?,
            Command::Write(addr, data) => {
                self.dbg.write_memory(*addr, *data)?;
                (*data).to_ne_bytes().to_vec()
            }
        };

        Ok(result)
    }
}
