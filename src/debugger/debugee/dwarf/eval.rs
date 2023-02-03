use crate::debugger;
use crate::debugger::debugee::dwarf::eval::EvalError::{OptionRequired, UnsupportedRequire};
use crate::debugger::debugee::dwarf::parser::unit::{DieVariant, Unit};
use crate::debugger::debugee::dwarf::EndianRcSlice;
use crate::debugger::register::get_register_value_dwarf;
use crate::debugger::RelocatedAddress;
use anyhow::anyhow;
use bytes::{BufMut, Bytes, BytesMut};
use gimli::{
    Encoding, EndianSlice, EvaluationResult, Expression, Location, Piece, RunTimeEndian,
    UnitOffset, Value, ValueType,
};
use nix::unistd::Pid;
use std::cmp::min;
use std::{mem, result};

#[derive(thiserror::Error, Debug)]
pub enum EvalError {
    #[error(transparent)]
    Gimli(#[from] gimli::read::Error),
    #[error("eval option {0} required")]
    OptionRequired(&'static str),
    #[error("unsupported evaluation require {0:?}")]
    UnsupportedRequire(String),
    #[error("nix error {0}")]
    Nix(nix::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = result::Result<T, EvalError>;

#[derive(Default)]
pub struct EvalOption<'a> {
    base_frame: Option<usize>,
    at_location: Option<Vec<u8>>,
    relocation_addr: Option<usize>,
    tls_resolver: Option<&'a dyn Fn(Pid, u64) -> anyhow::Result<RelocatedAddress>>,
}

impl<'a> EvalOption<'a> {
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

    pub fn with_relocation_addr(self, addr: usize) -> Self {
        Self {
            relocation_addr: Some(addr),
            ..self
        }
    }

    pub fn with_tls_resolver(
        self,
        resolver: &'a dyn Fn(Pid, u64) -> anyhow::Result<RelocatedAddress>,
    ) -> Self {
        Self {
            tls_resolver: Some(resolver),
            ..self
        }
    }
}

#[derive(Debug)]
pub struct ExpressionEvaluator<'a> {
    encoding: Encoding,
    unit: &'a Unit,
    pid: Pid,
}

impl<'a> ExpressionEvaluator<'a> {
    pub fn new(unit: &'a Unit, encoding: Encoding, pid: Pid) -> Self {
        Self {
            encoding,
            unit,
            pid,
        }
    }

    pub fn evaluate(&self, expr: Expression<EndianRcSlice>) -> Result<CompletedResult> {
        self.evaluate_with_opts(expr, EvalOption::default())
    }

    pub fn evaluate_with_opts(
        &self,
        expr: Expression<EndianRcSlice>,
        mut opts: EvalOption,
    ) -> Result<CompletedResult> {
        let mut eval = expr.evaluation(self.encoding);

        let mut result = eval.evaluate()?;
        while result != EvaluationResult::Complete {
            match result {
                EvaluationResult::RequiresRegister {
                    register,
                    base_type,
                } => {
                    let bytes = get_register_value_dwarf(self.pid, register.0 as i32)?;
                    let value_type = self.value_type_from_offset(base_type);
                    result = eval.resume_with_register(Value::from_u64(value_type, bytes)?)?;
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
                EvaluationResult::RequiresBaseType(offset) => {
                    let value_type = self.value_type_from_offset(offset);
                    result = eval.resume_with_base_type(value_type)?;
                }
                EvaluationResult::RequiresMemory {
                    address,
                    size,
                    base_type,
                    ..
                } => {
                    let memory =
                        debugger::read_memory_by_pid(self.pid, address as usize, size as usize)
                            .map_err(EvalError::Nix)?;

                    let value_type = self.value_type_from_offset(base_type);
                    let value = match value_type {
                        ValueType::Generic => {
                            let u = u64::from_ne_bytes(
                                memory.try_into().map_err(|e| anyhow!("{e:?}"))?,
                            );
                            Value::Generic(u)
                        }
                        _ => Value::parse(
                            value_type,
                            EndianSlice::new(&memory, RunTimeEndian::default()),
                        )?,
                    };

                    result = eval.resume_with_memory(value)?;
                }
                EvaluationResult::RequiresRelocatedAddress(addr) => {
                    let relocation_addr = opts
                        .relocation_addr
                        .ok_or(OptionRequired("relocation_addr"))?;
                    result = eval.resume_with_relocated_address(addr + relocation_addr as u64)?;
                }
                EvaluationResult::RequiresTls(offset) => {
                    let tls_resolver = opts
                        .tls_resolver
                        .take()
                        .ok_or(OptionRequired("tls_resolver"))?;
                    let addr = tls_resolver(self.pid, offset)?;
                    result = eval.resume_with_tls(addr.0 as u64)?;
                }
                _ => {
                    return Err(UnsupportedRequire(format!("{:?}", result)));
                }
            };
        }

        Ok(CompletedResult {
            inner: eval.result(),
            pid: self.pid,
        })
    }

    fn value_type_from_offset(&self, base_type: UnitOffset) -> ValueType {
        if base_type == UnitOffset(0) {
            ValueType::Generic
        } else {
            self.unit
                .find_entry(base_type)
                .and_then(|entry| match entry.die {
                    DieVariant::BaseType(ref bt_die) => Some(bt_die),
                    _ => None,
                })
                .and_then(|bt_die| Some((bt_die.byte_size?, bt_die.encoding?)))
                .and_then(|(size, encoding)| ValueType::from_encoding(encoding, size))
                .unwrap_or(ValueType::Generic)
        }
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
        let mut buf = BytesMut::with_capacity(byte_size);
        self.inner.into_iter().try_for_each(|piece| -> Result<()> {
            let read_size = piece
                .size_in_bits
                .map(|bits| bits as usize / 8)
                .unwrap_or(byte_size);
            let offset = piece.bit_offset.unwrap_or(0);

            match piece.location {
                Location::Register { register } => {
                    buf.put(read_register(
                        self.pid,
                        register.0 as i32,
                        read_size,
                        offset,
                    )?);
                }
                Location::Address { address } => {
                    let memory =
                        debugger::read_memory_by_pid(self.pid, address as usize, read_size)
                            .map_err(EvalError::Nix)?;
                    buf.put(Bytes::from(memory));
                }
                Location::Value { value } => {
                    match value {
                        Value::Generic(v) | Value::U64(v) => {
                            buf.put_u64(v);
                        }
                        Value::I8(v) => buf.put_i8(v),
                        Value::U8(v) => buf.put_u8(v),
                        Value::I16(v) => buf.put_i16(v),
                        Value::U16(v) => buf.put_u16(v),
                        Value::I32(v) => buf.put_i32(v),
                        Value::U32(v) => buf.put_u32(v),
                        Value::I64(v) => buf.put_i64(v),
                        Value::F32(v) => buf.put_f32(v),
                        Value::F64(v) => buf.put_f64(v),
                    };
                }
                Location::Bytes { value, .. } => {
                    buf.put_slice(value.bytes());
                }
                Location::ImplicitPointer { .. } => {
                    todo!()
                }
                Location::Empty => {}
            };
            Ok(())
        })?;

        Ok(buf.freeze())
    }
}

fn read_register(pid: Pid, reg_num: i32, size_in_bytes: usize, offset: u64) -> Result<Bytes> {
    let register_value = get_register_value_dwarf(pid, reg_num)?;
    let bytes = (register_value >> offset).to_ne_bytes();
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
