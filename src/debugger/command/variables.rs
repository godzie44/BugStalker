use crate::debugger::variable::VariableIR;
use crate::debugger::{command, Debugger, ReadModifier};
use std::collections::HashSet;

impl From<char> for ReadModifier {
    fn from(value: char) -> Self {
        match value {
            '*' => ReadModifier::Deref,
            _ => unreachable!(),
        }
    }
}

fn parse_path_and_modifiers(arg: &str) -> (Vec<ReadModifier>, String) {
    let mut result_path = String::new();
    let mut result_modifiers = vec![];

    let tokens = HashSet::from(['*']);

    for (idx, c) in arg.chars().enumerate() {
        if tokens.contains(&c) {
            result_modifiers.push(ReadModifier::from(c));
        } else {
            let (_, name) = arg.split_at(idx);
            result_path = name.to_string();
            break;
        }
    }

    (result_modifiers, result_path)
}

pub struct Variables<'a> {
    dbg: &'a Debugger,
    path: Option<(Vec<ReadModifier>, String)>,
}

impl<'a> Variables<'a> {
    pub fn new(debugger: &'a Debugger, args: Vec<&'a str>) -> command::Result<Self> {
        command::helper::check_args_count(&args, 1)?;
        Ok(Self {
            dbg: debugger,
            path: args.get(1).map(|s| parse_path_and_modifiers(s)),
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
            Some((modifiers, path)) => Ok(self.dbg.read_variable(&path, &modifiers)?),
        }
    }
}
