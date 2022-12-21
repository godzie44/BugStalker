use crate::debugger::command::CommandError;
use crate::debugger::{command, Debugger, EventHook};
use anyhow::anyhow;
use nix::libc::uintptr_t;
use std::mem;

enum SubCommand {
    Read(usize),
    Write(usize, uintptr_t),
}

pub struct Memory<'a, T: EventHook> {
    dbg: &'a Debugger<T>,
    sub_cmd: SubCommand,
}

impl<'a, T: EventHook> Memory<'a, T> {
    pub fn new<'s>(debugger: &'a Debugger<T>, args: Vec<&'s str>) -> command::Result<Self> {
        let sub_cmd = match args[1].to_lowercase().as_str() {
            "read" => {
                command::helper::check_args_count(&args, 3)?;

                SubCommand::Read(
                    usize::from_str_radix(args[2], 16)
                        .map_err(|e| CommandError::InvalidArgumentsEx(e.to_string()))?,
                )
            }
            "write" => {
                command::helper::check_args_count(&args, 4)?;
                SubCommand::Write(
                    usize::from_str_radix(args[2], 16)
                        .map_err(|e| CommandError::InvalidArgumentsEx(e.to_string()))?,
                    uintptr_t::from_str_radix(args[3], 16)
                        .map_err(|e| CommandError::InvalidArgumentsEx(e.to_string()))?,
                )
            }
            _ => return Err(CommandError::InvalidArguments),
        };

        Ok(Self {
            dbg: debugger,
            sub_cmd,
        })
    }

    pub fn run(&self) -> command::Result<uintptr_t> {
        let result = match &self.sub_cmd {
            SubCommand::Read(addr) => {
                let bytes = self
                    .dbg
                    .read_memory(*addr, mem::size_of::<usize>())
                    .map_err(anyhow::Error::from)?;
                uintptr_t::from_ne_bytes(bytes.try_into().map_err(|e| anyhow!("{e:?}"))?)
            }
            SubCommand::Write(addr, ptr) => {
                self.dbg
                    .write_memory(*addr, *ptr)
                    .map_err(anyhow::Error::from)?;
                *ptr
            }
        };

        Ok(result)
    }
}
