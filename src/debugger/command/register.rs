use crate::debugger::command::CommandError;
use crate::debugger::{command, register, Debugger};

enum SubCommand {
    Dump,
    Read(String),
    Write(String, u64),
}

pub struct Register<'a> {
    dbg: &'a Debugger,
    sub_cmd: SubCommand,
}

pub struct RegisterValue<'a> {
    pub register_name: &'a str,
    pub value: u64,
}

pub type Response<'a> = Vec<RegisterValue<'a>>;

impl<'a> Register<'a> {
    pub fn new<'s>(debugger: &'a Debugger, args: Vec<&'s str>) -> command::Result<Self> {
        command::helper::check_args_count(&args, 2)?;

        let sub_cmd = match args[1].to_lowercase().as_str() {
            "dump" => SubCommand::Dump,
            "read" => {
                command::helper::check_args_count(&args, 3)?;
                SubCommand::Read(args[2].to_string())
            }
            "write" => {
                command::helper::check_args_count(&args, 4)?;
                SubCommand::Write(
                    args[2].to_string(),
                    u64::from_str_radix(args[3], 16)
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

    pub fn run(&self) -> command::Result<Response> {
        match &self.sub_cmd {
            SubCommand::Dump => register::LIST
                .iter()
                .map(|descr| {
                    Ok(RegisterValue {
                        register_name: descr.name,
                        value: self.dbg.get_register_value(descr.name)?,
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()
                .map_err(|e| e.into()),
            SubCommand::Read(register) => Ok(vec![RegisterValue {
                register_name: register,
                value: self.dbg.get_register_value(register)?,
            }]),
            SubCommand::Write(register, value) => {
                self.dbg.set_register_value(register, *value)?;
                Ok(vec![])
            }
        }
    }
}
