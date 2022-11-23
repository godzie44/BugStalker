use crate::debugger::dwarf::eval::EvalError::{OptionRequired, UnsupportedRequire};
use crate::debugger::dwarf::EndianRcSlice;
use crate::debugger::register::get_register_value_dwarf;
use bytes::{BufMut, Bytes, BytesMut};
use gimli::{Encoding, EvaluationResult, Expression, Location, Piece, RunTimeEndian, Value};
use nix::sys;
use nix::sys::ptrace::AddressType;
use nix::unistd::Pid;
use std::cmp::min;
use std::ffi::c_long;
use std::{mem, result};

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
    at_location: Option<Vec<u8>>,
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

    pub fn with_at_location(self, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            at_location: Some(bytes.into()),
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
        mut opts: EvalOption,
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
                EvaluationResult::RequiresAtLocation(_) => {
                    let buf = EndianRcSlice::new(
                        opts.at_location
                            .take()
                            .ok_or(OptionRequired("at_location"))?
                            .into(),
                        RunTimeEndian::Little,
                    );
                    result = eval.resume_with_at_location(buf)?;
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
    pub fn into_scalar<T: Copy>(self) -> Result<T> {
        let bytes = self.into_raw_buffer(mem::size_of::<T>())?;
        Ok(scalar_from_bytes(bytes))
    }

    pub fn into_raw_buffer(self, byte_size: usize) -> Result<Bytes> {
        let mut buff = BytesMut::with_capacity(byte_size);
        self.inner.into_iter().try_for_each(|piece| -> Result<()> {
            let read_size = piece
                .size_in_bits
                .map(|bits| bits as usize / 8)
                .unwrap_or(byte_size);
            let offset = piece.bit_offset.unwrap_or(0);

            match piece.location {
                Location::Register { register } => {
                    buff.put(read_register(
                        self.pid,
                        register.0 as i32,
                        read_size,
                        offset,
                    )?);
                }
                Location::Address { address } => {
                    buff.put(read_memory(self.pid, address as usize, read_size)?);
                }
                Location::Value { value } => {
                    match value {
                        Value::Generic(v) | Value::U64(v) => {
                            buff.put_u64(v);
                        }
                        Value::I8(v) => buff.put_i8(v),
                        Value::U8(v) => buff.put_u8(v),
                        Value::I16(v) => buff.put_i16(v),
                        Value::U16(v) => buff.put_u16(v),
                        Value::I32(v) => buff.put_i32(v),
                        Value::U32(v) => buff.put_u32(v),
                        Value::I64(v) => buff.put_i64(v),
                        Value::F32(v) => buff.put_f32(v),
                        Value::F64(v) => buff.put_f64(v),
                    };
                }
                Location::Bytes { value, .. } => {
                    buff.put_slice(value.bytes());
                }
                Location::ImplicitPointer { .. } => {
                    todo!()
                }
                Location::Empty => {}
            };
            Ok(())
        })?;
        Ok(buff.freeze())
    }
}

fn u64_to_bytes(val: u64) -> [u8; 8] {
    #[cfg(target_endian = "big")]
    return val.to_be_bytes();
    #[cfg(target_endian = "little")]
    val.to_le_bytes()
}

fn read_memory(pid: Pid, address: usize, size_in_bytes: usize) -> Result<Bytes> {
    let mut buff = BytesMut::with_capacity(size_in_bytes);
    let mut address = address;
    let mut bytes_to_write = size_in_bytes;
    while bytes_to_write > 0 {
        let mem = sys::ptrace::read(pid, address as AddressType).map_err(EvalError::Nix)?;
        let bytes = u64_to_bytes(mem as u64);

        let write_size = min(bytes_to_write, std::mem::size_of::<u64>());
        buff.put_slice(&bytes[..write_size]);
        bytes_to_write -= write_size;
        address += mem::size_of::<c_long>()
    }

    Ok(buff.freeze())
}

fn read_register(pid: Pid, reg_num: i32, size_in_bytes: usize, offset: u64) -> Result<Bytes> {
    let register_value = get_register_value_dwarf(pid, reg_num)?;
    let bytes = u64_to_bytes(register_value >> offset);
    let write_size = min(size_in_bytes, std::mem::size_of::<u64>());
    Ok(Bytes::copy_from_slice(&bytes[..write_size]))
}

fn scalar_from_bytes<T: Copy>(bytes: Bytes) -> T {
    let ptr = bytes.as_ptr();
    if (ptr as usize) % mem::align_of::<T>() != 0 {
        panic!("invalid type alignment");
    }
    unsafe { *ptr.cast() }
}
