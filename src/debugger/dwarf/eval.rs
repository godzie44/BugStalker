use crate::debugger::dwarf::eval::EvalError::{OptionRequired, UnsupportedRequire};
use crate::debugger::dwarf::EndianRcSlice;
use crate::debugger::register::get_register_value_dwarf;
use gimli::{Encoding, EvaluationResult, Expression, Location, Piece, Value};
use nix::sys;
use nix::sys::ptrace::AddressType;
use nix::unistd::Pid;
use std::result;

#[derive(thiserror::Error, Debug)]
pub enum EvalError {
    #[error(transparent)]
    Gimli(#[from] gimli::read::Error),
    #[error("eval option {0} required")]
    OptionRequired(&'static str),
    #[error("unsupported evaluation require {0:?}")]
    UnsupportedRequire(EvaluationResult<EndianRcSlice>),
    #[error("nix error {0}")]
    Nix(nix::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

unsafe impl Send for EvalError {}
unsafe impl Sync for EvalError {}

pub type Result<T> = result::Result<T, EvalError>;

#[derive(Default)]
pub struct EvalOption {
    base_frame: Option<usize>,
}

impl EvalOption {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_base_frame(self, base_frame: usize) -> Self {
        Self {
            base_frame: Some(base_frame),
            ..self
        }
    }
}

#[derive(Debug)]
pub struct ExpressionEvaluator {
    encoding: Encoding,
}

impl ExpressionEvaluator {
    pub fn new(encoding: Encoding) -> Self {
        Self { encoding }
    }

    pub fn evaluate(&self, expr: Expression<EndianRcSlice>, pid: Pid) -> Result<CompletedResult> {
        self.evaluate_with_opts(expr, pid, EvalOption::default())
    }

    pub fn evaluate_with_opts(
        &self,
        expr: Expression<EndianRcSlice>,
        pid: Pid,
        opts: EvalOption,
    ) -> Result<CompletedResult> {
        let mut eval = expr.evaluation(self.encoding);

        let mut result = eval.evaluate()?;
        while result != EvaluationResult::Complete {
            match result {
                EvaluationResult::RequiresRegister {
                    register,
                    base_type: _base_type,
                } => {
                    let val = Value::Generic(get_register_value_dwarf(pid, register.0 as i32)?);
                    result = eval.resume_with_register(val)?;
                }
                EvaluationResult::RequiresFrameBase => {
                    result = eval.resume_with_frame_base(
                        opts.base_frame.ok_or(OptionRequired("base_frame"))? as u64,
                    )?;
                }
                EvaluationResult::RequiresMemory {
                    address,
                    size,
                    space: _,
                    base_type: _,
                } => {
                    println!("req mem {address} {size} ");
                    break;
                }
                _ => return Err(UnsupportedRequire(result)),
            };
        }

        Ok(CompletedResult {
            inner: eval.result(),
            pid,
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
            Location::Address { address } => sys::ptrace::read(self.pid, address as AddressType)
                .map(|v| v as u64)
                .map_err(EvalError::Nix),
            _ => {
                todo!()
            }
        }
    }
}
