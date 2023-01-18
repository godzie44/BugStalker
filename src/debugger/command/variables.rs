use crate::debugger::variable::VariableIR;
use crate::debugger::{command, Debugger, EventHook};

pub struct Variables<'a, T: EventHook> {
    dbg: &'a Debugger<T>,
    name: Option<String>,
}

impl<'a, T: EventHook> Variables<'a, T> {
    pub fn new(debugger: &'a Debugger<T>, args: Vec<&'a str>) -> command::Result<Self> {
        command::helper::check_args_count(&args, 1)?;
        Ok(Self {
            dbg: debugger,
            name: args.get(1).map(|s| s.to_string()),
        })
    }

    pub fn new_locals(debugger: &'a Debugger<T>) -> Self {
        Self {
            dbg: debugger,
            name: None,
        }
    }

    pub fn run(&self) -> command::Result<Vec<VariableIR>> {
        match self.name {
            None => Ok(self.dbg.read_local_variables()?),
            Some(ref name) => Ok(self.dbg.read_variable(name)?),
        }
    }
}
