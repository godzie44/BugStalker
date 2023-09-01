use crate::debugger::command::HandlingError;
use crate::debugger::register::RegisterMap;
use crate::debugger::{command, register, Debugger};
use register::Register as Reg;

#[derive(Debug)]
pub enum Command {
    Info,
    Read(String),
    Write(String, u64),
}

pub struct Register<'a> {
    dbg: &'a Debugger,
}

pub struct RegisterValue {
    pub register_name: String,
    pub value: u64,
}

pub type Response = Vec<RegisterValue>;

impl<'a> Register<'a> {
    pub fn new(debugger: &'a Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(self, cmd: &Command) -> command::HandleResult<Response> {
        match cmd {
            Command::Info => {
                let registers_to_dump = &[
                    Reg::Rax,
                    Reg::Rbx,
                    Reg::Rcx,
                    Reg::Rdx,
                    Reg::Rdi,
                    Reg::Rsi,
                    Reg::Rbp,
                    Reg::Rsp,
                    Reg::R8,
                    Reg::R9,
                    Reg::R10,
                    Reg::R11,
                    Reg::R12,
                    Reg::R13,
                    Reg::R14,
                    Reg::R15,
                    Reg::Rip,
                    Reg::Eflags,
                    Reg::Cs,
                    Reg::OrigRax,
                    Reg::FsBase,
                    Reg::GsBase,
                    Reg::Fs,
                    Reg::Gs,
                    Reg::Ss,
                    Reg::Ds,
                    Reg::Es,
                ];

                let register_map = RegisterMap::current(self.dbg.exploration_ctx().pid_on_focus())
                    .map_err(|e| HandlingError::Debugger(e.into()))?;

                Ok(registers_to_dump
                    .iter()
                    .map(|&r| RegisterValue {
                        register_name: r.to_string(),
                        value: register_map.value(r),
                    })
                    .collect::<Vec<_>>())
            }
            Command::Read(register) => Ok(vec![RegisterValue {
                register_name: register.to_string(),
                value: self.dbg.get_register_value(register)?,
            }]),
            Command::Write(register, value) => {
                self.dbg.set_register_value(register, *value)?;
                Ok(vec![])
            }
        }
    }
}
