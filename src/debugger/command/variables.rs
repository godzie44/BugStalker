use crate::debugger::command::expression::{ExprPlan, ExprPlanParser};
use crate::debugger::command::CommandError::ParseArgument;
use crate::debugger::variable::VariableIR;
use crate::debugger::{command, Debugger};

pub struct Variables<'a> {
    dbg: &'a Debugger,
    path: Option<ExprPlan>,
}

impl<'a> Variables<'a> {
    pub fn new(debugger: &'a Debugger, args: Vec<&'a str>) -> command::Result<Self> {
        command::helper::check_args_count(&args, 1)?;
        Ok(Self {
            dbg: debugger,
            path: args
                .get(1)
                .map(|s| {
                    let parser = ExprPlanParser::new(s);
                    parser.parse().map_err(ParseArgument)
                })
                .transpose()?,
        })
    }

    pub fn new_locals(debugger: &'a Debugger) -> Self {
        Self {
            dbg: debugger,
            path: None,
        }
    }

    pub fn run(self) -> command::Result<Vec<VariableIR>> {
        match self.path {
            None => Ok(self.dbg.read_local_variables()?),
            Some(expr) => Ok(self.dbg.read_variable(expr)?),
        }
    }
}
