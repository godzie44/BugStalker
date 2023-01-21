use crate::debugger::command::CommandError;
use crate::debugger::{command, Debugger, PCValue, RelocatedAddress};
use std::str::FromStr;

pub enum Breakpoint {
    Address(usize),
    Line(String, u64),
    Function(String),
}

pub struct Break<'a> {
    dbg: &'a mut Debugger,
    pub r#type: Breakpoint,
}

impl<'a> Break<'a> {
    pub fn new<'s>(debugger: &'a mut Debugger, args: Vec<&'s str>) -> command::Result<Self> {
        command::helper::check_args_count(&args, 2)?;

        let break_point_place = args[1];
        let break_point_type;
        if break_point_place.starts_with("0x") {
            let addr = usize::from_str_radix(&break_point_place[2..], 16)
                .map_err(|e| CommandError::InvalidArgumentsEx(e.to_string()))?;
            break_point_type = Breakpoint::Address(addr);
        } else if break_point_place.find(':').is_some() {
            let args = break_point_place.split(':').collect::<Vec<_>>();
            break_point_type = Breakpoint::Line(
                args[0].to_string(),
                u64::from_str(args[1])
                    .map_err(|e| CommandError::InvalidArgumentsEx(e.to_string()))?,
            );
        } else {
            break_point_type = Breakpoint::Function(break_point_place.to_string())
        }

        Ok(Self {
            dbg: debugger,
            r#type: break_point_type,
        })
    }

    pub fn run(&mut self) -> command::Result<()> {
        match &self.r#type {
            Breakpoint::Address(addr) => Ok(self
                .dbg
                .set_breakpoint(PCValue::Relocated(RelocatedAddress(*addr)))?),
            Breakpoint::Line(file, line) => Ok(self.dbg.set_breakpoint_at_line(file, *line)?),
            Breakpoint::Function(func_name) => Ok(self.dbg.set_breakpoint_at_fn(func_name)?),
        }
    }
}
