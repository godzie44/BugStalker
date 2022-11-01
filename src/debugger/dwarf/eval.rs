use crate::debugger::dwarf::eval::EvalError::UnsupportedRequire;
use crate::debugger::dwarf::{DwarfContext, EndianRcSlice, ParsedUnit};
use crate::debugger::register::get_register_value_dwarf;
use gimli::{EvaluationResult, Expression, Location, Piece, Value};
use nix::unistd::Pid;
use std::result;

#[derive(thiserror::Error, Debug)]
pub enum EvalError {
    #[error(transparent)]
    Gimli(#[from] gimli::read::Error),
    #[error("unsupported evaluation require {0:?}")]
    UnsupportedRequire(EvaluationResult<EndianRcSlice>),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

unsafe impl Send for EvalError {}
unsafe impl Sync for EvalError {}

pub type Result<T> = result::Result<T, EvalError>;

pub struct ExpressionEvaluator<'a> {
    _dwarf_ctx: &'a DwarfContext,
    pid: Pid,
}

impl<'a> ExpressionEvaluator<'a> {
    pub fn new(pid: Pid, dwarf: &'a DwarfContext) -> Self {
        Self {
            pid,
            _dwarf_ctx: dwarf,
        }
    }

    pub fn evaluate(
        &self,
        unit: &ParsedUnit,
        expr: Expression<EndianRcSlice>,
    ) -> Result<CompletedResult> {
        let mut eval = expr.evaluation(unit._unit.encoding());

        let mut result = eval.evaluate()?;
        while result != EvaluationResult::Complete {
            match result {
                EvaluationResult::RequiresRegister {
                    register,
                    base_type,
                } => {
                    let val =
                        Value::Generic(get_register_value_dwarf(self.pid, register.0 as i32)?);
                    result = eval.resume_with_register(val)?;
                }

                EvaluationResult::RequiresMemory {
                    address,
                    size,
                    space,
                    base_type,
                } => {
                    println!("req mem {address} {size} ");
                    break;
                }
                _ => return Err(UnsupportedRequire(result)),
            };
        }

        Ok(CompletedResult {
            inner: eval.result(),
            pid: self.pid,
        })
    }
}

pub struct CompletedResult {
    inner: Vec<Piece<EndianRcSlice>>,
    pid: Pid,
}

impl CompletedResult {
    pub fn as_u64(&self) -> Result<u64> {
        match self.inner[0].location {
            Location::Register { register } => {
                Ok(get_register_value_dwarf(self.pid, register.0 as i32)?)
            }
            _ => {
                todo!()
            }
        }
    }
}
