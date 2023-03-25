use crate::debugger::{command, register, Debugger};

#[derive(Debug)]
pub enum Command {
    Dump,
    Read(String),
    Write(String, u64),
}

pub struct Register<'a> {
    dbg: &'a Debugger,
}

pub struct RegisterValue<'a> {
    pub register_name: &'a str,
    pub value: u64,
}

pub type Response<'a> = Vec<RegisterValue<'a>>;

impl<'a> Register<'a> {
    pub fn new(debugger: &'a Debugger) -> Self {
        Self { dbg: debugger }
    }

    pub fn handle(self, cmd: &Command) -> command::HandleResult<Response> {
        match cmd {
            Command::Dump => register::LIST
                .iter()
                .map(|descr| {
                    Ok(RegisterValue {
                        register_name: descr.name,
                        value: self.dbg.get_register_value(descr.name)?,
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()
                .map_err(|e| e.into()),
            Command::Read(register) => Ok(vec![RegisterValue {
                register_name: register,
                value: self.dbg.get_register_value(register)?,
            }]),
            Command::Write(register, value) => {
                self.dbg.set_register_value(register, *value)?;
                Ok(vec![])
            }
        }
    }
}
