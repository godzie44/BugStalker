use crate::debugger;
use crate::debugger::address::{GlobalAddress, RelocatedAddress};
use crate::debugger::debugee::dwarf::eval::EvalError::{OptionRequired, UnsupportedRequire};
use crate::debugger::debugee::dwarf::unit::DieRef;
use crate::debugger::debugee::dwarf::unit::{DieVariant, Unit};
use crate::debugger::debugee::dwarf::{ContextualDieRef, DwarfUnwinder, EndianArcSlice};
use crate::debugger::debugee::Debugee;
use crate::debugger::register::{DwarfRegisterMap, RegisterMap};
use crate::debugger::unwind::libunwind;
use crate::debugger::{debugee, ExplorationContext};
use anyhow::anyhow;
use bytes::{BufMut, Bytes, BytesMut};
use gimli::{
    DebugAddr, Encoding, EndianSlice, EvaluationResult, Expression, Location, Piece, Register,
    RunTimeEndian, UnitOffset, Value, ValueType,
};
use nix::unistd::Pid;
use object::ReadRef;
use std::cell::RefCell;
use std::cmp::min;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
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
    Nix(#[from] nix::Error),
    #[error("unwind error {0}")]
    Unwind(#[from] unwind::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = result::Result<T, EvalError>;

/// Resolve requirements that the `ExpressionEvaluator` may need. Relevant for the current breakpoint.
/// Some options are lazy to avoid overhead on recalculation.
struct RequirementsResolver<'a> {
    debugee: &'a Debugee,
    cfa: RefCell<HashMap<Pid, RelocatedAddress>>,
    base_address: RefCell<HashMap<Pid, RelocatedAddress>>,
}

impl<'a> RequirementsResolver<'a> {
    fn new(debugee: &'a Debugee) -> Self {
        RequirementsResolver {
            debugee,
            cfa: RefCell::default(),
            base_address: RefCell::default(),
        }
    }

    /// Return base address of current frame.
    fn base_addr(&self, ctx: &ExplorationContext) -> anyhow::Result<RelocatedAddress> {
        match self.base_address.borrow_mut().entry(ctx.pid_on_focus()) {
            Entry::Occupied(e) => Ok(*e.get()),
            Entry::Vacant(e) => {
                let loc = ctx.location();
                let func = self
                    .debugee
                    .dwarf()
                    .find_function_by_pc(loc.global_pc)
                    .ok_or_else(|| anyhow!("current function not found"))?;
                let base_addr = func.frame_base_addr(ctx, self.debugee)?;
                Ok(*e.insert(base_addr))
            }
        }
    }

    /// Return canonical frame address of current frame.
    fn cfa(&self, ctx: &ExplorationContext) -> anyhow::Result<RelocatedAddress> {
        match self.cfa.borrow_mut().entry(ctx.pid_on_focus()) {
            Entry::Occupied(e) => Ok(*e.get()),
            Entry::Vacant(e) => {
                let cfa = self.debugee.dwarf().get_cfa(self.debugee, ctx)?;
                Ok(*e.insert(cfa))
            }
        }
    }

    fn relocation_addr(&self) -> usize {
        self.debugee.mapping_offset()
    }

    fn resolve_tls(&self, pid: Pid, offset: u64) -> anyhow::Result<RelocatedAddress> {
        let lm_addr = self.debugee.rendezvous().link_map_main();
        self.debugee
            .tracee_ctl()
            .tls_addr(pid, lm_addr, offset as usize)
    }

    fn debug_addr_section(&self) -> &DebugAddr<EndianArcSlice> {
        self.debugee.dwarf().debug_addr()
    }

    fn resolve_registers(&self, ctx: &ExplorationContext) -> anyhow::Result<DwarfRegisterMap> {
        let current_loc = ctx.location();
        let current_fn = self
            .debugee
            .dwarf()
            .find_function_by_pc(current_loc.global_pc)
            .ok_or_else(|| anyhow!("not in function"))?;
        let entry_pc: GlobalAddress = current_fn
            .die
            .base_attributes
            .ranges
            .iter()
            .map(|r| r.begin)
            .min()
            .ok_or_else(|| anyhow!("entry point pc not found"))?
            .into();

        let backtrace = libunwind::unwind(ctx.pid_on_focus())?;
        let entry_pc_rel = entry_pc.relocate(self.debugee.mapping_offset());
        backtrace
            .iter()
            .enumerate()
            .find(|(_, frame)| frame.fn_start_ip == Some(entry_pc_rel))
            .map(|(num, _)| -> anyhow::Result<DwarfRegisterMap> {
                // try to use libunwind if frame determined
                let mut registers = RegisterMap::current(ctx.pid_on_focus())?.into();
                libunwind::restore_registers_at_frame(
                    ctx.pid_on_focus(),
                    &mut registers,
                    num as u32,
                )?;
                Ok(registers)
            })
            .unwrap_or_else(|| {
                // use dwarf unwinder as a fallback
                let unwinder = DwarfUnwinder::new(self.debugee);
                let location = debugee::Location {
                    pid: ctx.pid_on_focus(),
                    pc: entry_pc_rel,
                    global_pc: entry_pc,
                };
                Ok(unwinder
                    .context_for(&ExplorationContext::new(location, ctx.focus_frame))?
                    .ok_or(anyhow!("fetch register fail"))?
                    .registers())
            })
    }
}

/// Resolve requirements that the `ExpressionEvaluator` may need.
/// Defined by callee, the composition of this requirements depends on the context of the calculation.
#[derive(Default)]
pub struct ExternalRequirementsResolver {
    at_location: Option<Vec<u8>>,
    entry_registers: HashMap<Pid, DwarfRegisterMap>,
}

impl ExternalRequirementsResolver {
    pub fn new() -> Self {
        Self {
            at_location: None,
            entry_registers: HashMap::default(),
        }
    }

    pub fn with_at_location(self, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            at_location: Some(bytes.into()),
            ..self
        }
    }

    pub fn with_entry_registers(self, pid: Pid, registers: DwarfRegisterMap) -> Self {
        let mut regs = self.entry_registers;
        regs.insert(pid, registers);
        Self {
            entry_registers: regs,
            ..self
        }
    }
}

pub struct ExpressionEvaluator<'a> {
    encoding: Encoding,
    unit: &'a Unit,
    resolver: RequirementsResolver<'a>,
}

impl<'a> ExpressionEvaluator<'a> {
    pub fn new(unit: &'a Unit, encoding: Encoding, debugee: &'a Debugee) -> Self {
        Self {
            encoding,
            unit,
            resolver: RequirementsResolver::new(debugee),
        }
    }

    fn value_type_from_offset(&self, base_type: UnitOffset) -> ValueType {
        if base_type == UnitOffset(0) {
            ValueType::Generic
        } else {
            self.unit
                .find_entry(base_type)
                .ensure_ok()
                .and_then(|entry| match entry.die {
                    DieVariant::BaseType(ref bt_die) => Some(bt_die),
                    _ => None,
                })
                .and_then(|bt_die| Some((bt_die.byte_size?, bt_die.encoding?)))
                .and_then(|(size, encoding)| ValueType::from_encoding(encoding, size))
                .unwrap_or(ValueType::Generic)
        }
    }

    pub fn evaluate(
        &self,
        ctx: &'a ExplorationContext,
        expr: Expression<EndianArcSlice>,
    ) -> Result<CompletedResult> {
        self.evaluate_with_resolver(ExternalRequirementsResolver::default(), ctx, expr)
    }

    pub fn evaluate_with_resolver(
        &self,
        mut resolver: ExternalRequirementsResolver,
        ctx: &'a ExplorationContext,
        expr: Expression<EndianArcSlice>,
    ) -> Result<CompletedResult> {
        let mut eval = expr.evaluation(self.encoding);

        let mut result = eval.evaluate()?;
        while result != EvaluationResult::Complete {
            match result {
                EvaluationResult::RequiresRegister {
                    register,
                    base_type,
                } => {
                    let value_type = self.value_type_from_offset(base_type);

                    // if there is registers dump for functions entry - use it
                    let bytes = if let Some(regs) =
                        resolver.entry_registers.remove(&ctx.pid_on_focus())
                    {
                        regs.value(register)?
                    } else {
                        let pid = ctx.pid_on_focus();
                        let mut registers = DwarfRegisterMap::from(RegisterMap::current(pid)?);
                        // try to use registers for in focus frame
                        libunwind::restore_registers_at_frame(pid, &mut registers, ctx.frame())?;
                        registers.value(register)?
                    };
                    result = eval.resume_with_register(Value::from_u64(value_type, bytes)?)?;
                }
                EvaluationResult::RequiresFrameBase => {
                    result = eval.resume_with_frame_base(self.resolver.base_addr(ctx)?.into())?;
                }
                EvaluationResult::RequiresAtLocation(_) => {
                    let buf = EndianArcSlice::new(
                        resolver
                            .at_location
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
                    let memory = debugger::read_memory_by_pid(
                        ctx.pid_on_focus(),
                        address as usize,
                        size as usize,
                    )
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
                    let relocation_addr = self.resolver.relocation_addr();
                    result = eval.resume_with_relocated_address(addr + relocation_addr as u64)?;
                }
                EvaluationResult::RequiresTls(offset) => {
                    let addr = self.resolver.resolve_tls(ctx.pid_on_focus(), offset)?;
                    result = eval.resume_with_tls(addr.into())?;
                }
                EvaluationResult::RequiresIndexedAddress { index, relocate } => {
                    let debug_addr = self.resolver.debug_addr_section();
                    let mut addr = debug_addr.get_address(
                        self.unit.address_size(),
                        self.unit.addr_base(),
                        index,
                    )?;
                    if relocate {
                        addr += self.resolver.relocation_addr() as u64;
                    }
                    result = eval.resume_with_indexed_address(addr)?;
                }
                EvaluationResult::RequiresCallFrameCfa => {
                    let cfa = self.resolver.cfa(ctx)?;
                    result = eval.resume_with_call_frame_cfa(cfa.into())?;
                }
                EvaluationResult::RequiresEntryValue(expr) => {
                    let regs = self.resolver.resolve_registers(ctx)?;
                    let ctx_resolver = ExternalRequirementsResolver::default()
                        .with_entry_registers(ctx.pid_on_focus(), regs);
                    let eval_res = self.evaluate_with_resolver(ctx_resolver, ctx, expr)?;
                    let u = eval_res.into_scalar::<u64>()?;
                    result = eval.resume_with_entry_value(Value::Generic(u))?;
                }
                EvaluationResult::RequiresParameterRef(_) => {
                    return Err(UnsupportedRequire(format!("{:?}", result)));
                }
                EvaluationResult::Complete => {
                    unreachable!()
                }
            };
        }

        Ok(CompletedResult {
            debugee: self.resolver.debugee,
            unit: self.unit,
            inner: eval.result(),
            ctx,
        })
    }
}

pub struct CompletedResult<'a> {
    inner: Vec<Piece<EndianArcSlice>>,
    debugee: &'a Debugee,
    unit: &'a Unit,
    ctx: &'a ExplorationContext,
}

impl<'a> CompletedResult<'a> {
    pub fn into_scalar<T: Copy>(self) -> Result<T> {
        let bytes = self.into_raw_buffer(mem::size_of::<T>())?;
        Ok(scalar_from_bytes(&bytes))
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
                    buf.put(read_register(self.ctx, register, read_size, offset)?);
                }
                Location::Address { address } => {
                    let memory = debugger::read_memory_by_pid(
                        self.ctx.pid_on_focus(),
                        address as usize,
                        read_size,
                    )
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
                Location::ImplicitPointer { value, byte_offset } => {
                    let die_ref = DieRef::Global(value);
                    let (entry, unit) = self
                        .debugee
                        .dwarf()
                        .deref_die(self.unit, die_ref)
                        .ok_or_else(|| {
                            EvalError::Other(anyhow!("die not found by ref: {die_ref:?}"))
                        })?;
                    if let DieVariant::Variable(ref variable) = &entry.die {
                        let ctx_die = ContextualDieRef {
                            context: self.debugee.dwarf(),
                            unit_idx: unit.idx(),
                            node: &entry.node,
                            die: variable,
                        };
                        let r#type = ctx_die
                            .r#type()
                            .ok_or_else(|| EvalError::Other(anyhow!("unknown die type")))?;
                        let bytes = ctx_die
                            .read_value(self.ctx, self.debugee, &r#type)
                            .ok_or_else(|| {
                                EvalError::Other(anyhow!("implicit pointer address invalid value"))
                            })?;
                        let bytes: &[u8] = bytes
                            .read_slice_at(byte_offset as u64, byte_size)
                            .map_err(|_| {
                                EvalError::Other(anyhow!("implicit pointer address invalid value"))
                            })?;
                        buf.put_slice(bytes)
                    }
                }
                Location::Empty => {}
            };
            Ok(())
        })?;

        Ok(buf.freeze())
    }
}

fn read_register(
    ctx: &ExplorationContext,
    reg: Register,
    size_in_bytes: usize,
    offset: u64,
) -> Result<Bytes> {
    let pid = ctx.pid_on_focus();
    let mut registers = DwarfRegisterMap::from(RegisterMap::current(pid)?);
    // try to use registers for in focus frame
    libunwind::restore_registers_at_frame(pid, &mut registers, ctx.frame())?;
    let register_value = registers.value(reg)?;
    let bytes = (register_value >> offset).to_ne_bytes();
    let write_size = min(size_in_bytes, std::mem::size_of::<u64>());
    Ok(Bytes::copy_from_slice(&bytes[..write_size]))
}

#[inline(never)]
fn scalar_from_bytes<T: Copy>(bytes: &Bytes) -> T {
    let ptr = bytes.as_ptr();
    unsafe { std::ptr::read_unaligned::<T>(ptr as *const T) }
}
