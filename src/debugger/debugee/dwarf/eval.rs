use crate::debugger;
use crate::debugger::address::{GlobalAddress, RelocatedAddress};
use crate::debugger::debugee::Debugee;
use crate::debugger::debugee::dwarf::unit::DieRef;
use crate::debugger::debugee::dwarf::unit::{DieVariant, Unit};
use crate::debugger::debugee::dwarf::{ContextualDieRef, DwarfUnwinder, EndianArcSlice};
use crate::debugger::error::Error;
use crate::debugger::error::Error::{
    DieNotFound, EvalOptionRequired, EvalUnsupportedRequire, FunctionNotFound, ImplicitPointer,
    NoDieType, Ptrace, TypeBinaryRepr, UnwindNoContext,
};
use crate::debugger::register::{DwarfRegisterMap, RegisterMap};
use crate::debugger::{ExplorationContext, debugee};
use crate::version::RustVersion;
use bytes::{BufMut, Bytes, BytesMut};
use gimli::{
    DebugAddr, Encoding, EndianSlice, EvaluationResult, Expression, Location, Piece, Register,
    RunTimeEndian, UnitOffset, Value, ValueType,
};
use nix::unistd::Pid;
use object::ReadRef;
use std::cell::RefCell;
use std::cmp::min;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::mem;

pub struct EvaluationContext<'a> {
    pub evaluator: &'a ExpressionEvaluator<'a>,
    pub expl_ctx: &'a ExplorationContext,
}

impl EvaluationContext<'_> {
    pub fn rustc_version(&self) -> Option<RustVersion> {
        self.evaluator.unit().rustc_version()
    }
}

/// Resolve requirements that the `ExpressionEvaluator` may need. Relevant for the current breakpoint.
/// Some options are lazy to avoid overhead on recalculation.
#[derive(Clone)]
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
    fn base_addr(&self, ctx: &ExplorationContext) -> Result<RelocatedAddress, Error> {
        match self.base_address.borrow_mut().entry(ctx.pid_on_focus()) {
            Entry::Occupied(e) => Ok(*e.get()),
            Entry::Vacant(e) => {
                let loc = ctx.location();
                let func = self
                    .debugee
                    .debug_info(ctx.location().pc)?
                    .find_function_by_pc(loc.global_pc)?
                    .ok_or(FunctionNotFound(loc.global_pc))?;
                let base_addr = func.frame_base_addr(ctx, self.debugee)?;
                Ok(*e.insert(base_addr))
            }
        }
    }

    /// Return canonical frame address of current frame.
    fn cfa(&self, ctx: &ExplorationContext) -> Result<RelocatedAddress, Error> {
        match self.cfa.borrow_mut().entry(ctx.pid_on_focus()) {
            Entry::Occupied(e) => Ok(*e.get()),
            Entry::Vacant(e) => {
                let cfa = self
                    .debugee
                    .debug_info(ctx.location().pc)?
                    .get_cfa(self.debugee, ctx)?;
                Ok(*e.insert(cfa))
            }
        }
    }

    fn relocation_addr(&self, ctx: &ExplorationContext) -> Result<usize, Error> {
        self.debugee.mapping_offset_for_pc(ctx.location().pc)
    }

    fn resolve_tls(&self, pid: Pid, offset: u64) -> Result<RelocatedAddress, Error> {
        let lm_addr = self.debugee.rendezvous().link_map_main();
        self.debugee
            .tracee_ctl()
            .tls_addr(pid, lm_addr, offset as usize)
    }

    fn debug_addr_section(
        &self,
        ctx: &ExplorationContext,
    ) -> Result<&DebugAddr<EndianArcSlice>, Error> {
        Ok(self.debugee.debug_info(ctx.location().pc)?.debug_addr())
    }

    fn resolve_registers(&self, ctx: &ExplorationContext) -> Result<DwarfRegisterMap, Error> {
        let current_loc = ctx.location();
        let current_fn = self
            .debugee
            .debug_info(ctx.location().pc)?
            .find_function_by_pc(current_loc.global_pc)?
            .ok_or(FunctionNotFound(current_loc.global_pc))?;
        let entry_pc: GlobalAddress = current_fn.start_instruction()?;

        let backtrace = self.debugee.unwind(ctx.pid_on_focus())?;
        let entry_pc_rel = entry_pc.relocate_to_segment_by_pc(self.debugee, ctx.location().pc)?;
        backtrace
            .iter()
            .enumerate()
            .find(|(_, frame)| frame.fn_start_ip == Some(entry_pc_rel))
            .map(|(num, _)| -> Result<DwarfRegisterMap, Error> {
                // try to use libunwind if frame determined
                let mut registers = RegisterMap::current(ctx.pid_on_focus())?.into();
                self.debugee.restore_registers_at_frame(
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
                    .context_for(&ExplorationContext::new(location, ctx.frame_num()))?
                    .ok_or(UnwindNoContext)?
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

#[derive(Clone)]
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

    pub fn unit(&self) -> &Unit {
        self.unit
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
    ) -> Result<CompletedResult<'_>, Error> {
        self.evaluate_with_resolver(ExternalRequirementsResolver::default(), ctx, expr)
    }

    pub fn evaluate_with_resolver(
        &self,
        mut resolver: ExternalRequirementsResolver,
        ctx: &'a ExplorationContext,
        expr: Expression<EndianArcSlice>,
    ) -> Result<CompletedResult<'_>, Error> {
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
                    let bytes =
                        if let Some(regs) = resolver.entry_registers.remove(&ctx.pid_on_focus()) {
                            regs.value(register)?
                        } else {
                            let pid = ctx.pid_on_focus();
                            let mut registers = DwarfRegisterMap::from(RegisterMap::current(pid)?);
                            // try to use registers for in focus frame
                            self.resolver.debugee.restore_registers_at_frame(
                                ctx.pid_on_focus(),
                                &mut registers,
                                ctx.frame_num(),
                            )?;
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
                            .ok_or(EvalOptionRequired("at_location"))?
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
                    .map_err(Ptrace)?;

                    let value_type = self.value_type_from_offset(base_type);
                    let value = match value_type {
                        ValueType::Generic => {
                            let u =
                                u64::from_ne_bytes(memory.try_into().map_err(|data: Vec<_>| {
                                    TypeBinaryRepr("u64", data.into_boxed_slice())
                                })?);
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
                    let relocation_addr = self.resolver.relocation_addr(ctx)?;
                    result = eval.resume_with_relocated_address(addr + relocation_addr as u64)?;
                }
                EvaluationResult::RequiresTls(offset) => {
                    let addr = self.resolver.resolve_tls(ctx.pid_on_focus(), offset)?;
                    result = eval.resume_with_tls(addr.into())?;
                }
                EvaluationResult::RequiresIndexedAddress { index, relocate } => {
                    let debug_addr = self.resolver.debug_addr_section(ctx)?;
                    let mut addr = debug_addr.get_address(
                        self.unit.address_size(),
                        self.unit.addr_base(),
                        index,
                    )?;
                    if relocate {
                        addr += self.resolver.relocation_addr(ctx)? as u64;
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
                    let u = eval_res.into_scalar::<u64>(AddressKind::MemoryAddress)?;
                    result = eval.resume_with_entry_value(Value::Generic(u))?;
                }
                EvaluationResult::RequiresParameterRef(_) => {
                    return Err(EvalUnsupportedRequire("parameter_ref"));
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

/// Determine how to interpret [`Location::Address`] in piece location field,
/// as value or as address in debugee memory.
/// This information is context dependent (depending on what exactly is being calculated).
///
/// This code may be deleted in future, see https://github.com/gimli-rs/gimli/issues/675 and
/// https://github.com/gimli-rs/gimli/pull/676
pub enum AddressKind {
    MemoryAddress,
    Value,
}

impl CompletedResult<'_> {
    pub fn into_scalar<T: Copy>(self, address_kind: AddressKind) -> Result<T, Error> {
        let (_, bytes) = self.into_raw_bytes(mem::size_of::<T>(), address_kind)?;
        Ok(scalar_from_bytes(&bytes))
    }

    /// Return a value selected from DWARF expression as raw bytes, and an address of this value
    /// in debugee memory (if possible).
    ///
    /// # Arguments
    ///
    /// * `byte_size`: length of expected value in bytes
    /// * `address_kind`: determine how to interpret [`Location::Address`] in piece location field
    pub fn into_raw_bytes(
        self,
        byte_size: usize,
        address_kind: AddressKind,
    ) -> Result<(Option<usize>, Bytes), Error> {
        let mut data = BytesMut::with_capacity(byte_size);
        let mut data_addr = None;

        self.inner
            .into_iter()
            .enumerate()
            .try_for_each(|(i, piece)| -> Result<(), Error> {
                let read_size = piece
                    .size_in_bits
                    .map(|bits| bits as usize / 8)
                    .unwrap_or(byte_size);
                let offset = piece.bit_offset.unwrap_or(0);

                match piece.location {
                    Location::Register { register } => {
                        data.put(read_register(
                            self.debugee,
                            self.ctx,
                            register,
                            read_size,
                            offset,
                        )?);
                    }
                    Location::Address { address } => {
                        if i == 0 {
                            data_addr = Some(address as usize);
                        }
                        match address_kind {
                            AddressKind::MemoryAddress => {
                                let memory = debugger::read_memory_by_pid(
                                    self.ctx.pid_on_focus(),
                                    address as usize,
                                    read_size,
                                )
                                .map_err(Ptrace)?;
                                data.put(Bytes::from(memory))
                            }
                            AddressKind::Value => {
                                data.put_slice(&address.to_ne_bytes());
                            }
                        };
                    }
                    Location::Value { value } => {
                        match value {
                            Value::Generic(v) | Value::U64(v) => data.put_slice(&v.to_ne_bytes()),
                            Value::I8(v) => data.put_slice(&v.to_ne_bytes()),
                            Value::U8(v) => data.put_slice(&v.to_ne_bytes()),
                            Value::I16(v) => data.put_slice(&v.to_ne_bytes()),
                            Value::U16(v) => data.put_slice(&v.to_ne_bytes()),
                            Value::I32(v) => data.put_slice(&v.to_ne_bytes()),
                            Value::U32(v) => data.put_slice(&v.to_ne_bytes()),
                            Value::I64(v) => data.put_slice(&v.to_ne_bytes()),
                            Value::F32(v) => data.put_slice(&v.to_ne_bytes()),
                            Value::F64(v) => data.put_slice(&v.to_ne_bytes()),
                        };
                    }
                    Location::Bytes { value, .. } => {
                        data.put_slice(value.bytes());
                    }
                    Location::ImplicitPointer { value, byte_offset } => {
                        let die_ref = DieRef::Global(value);
                        let dwarf = self.debugee.debug_info(self.ctx.location().pc)?;
                        let (entry, unit) = dwarf
                            .deref_die(self.unit, die_ref)
                            .ok_or_else(|| DieNotFound(die_ref))?;
                        if let DieVariant::Variable(variable) = &entry.die {
                            let ctx_die = ContextualDieRef {
                                debug_info: dwarf,
                                unit_idx: unit.idx(),
                                node: &entry.node,
                                die: variable,
                            };
                            let r#type = ctx_die.r#type().ok_or(NoDieType)?;
                            let repr = ctx_die
                                .read_value(self.ctx, self.debugee, &r#type)
                                .ok_or(ImplicitPointer)?;
                            let bytes: &[u8] = repr
                                .raw_data
                                .read_slice_at(byte_offset as u64, byte_size)
                                .map_err(|_| ImplicitPointer)?;
                            data.put_slice(bytes)
                        }
                    }
                    Location::Empty => {}
                };
                Ok(())
            })?;

        Ok((data_addr, data.freeze()))
    }
}

fn read_register(
    debugee: &Debugee,
    ctx: &ExplorationContext,
    reg: Register,
    size_in_bytes: usize,
    offset: u64,
) -> Result<Bytes, Error> {
    let pid = ctx.pid_on_focus();
    let mut registers = DwarfRegisterMap::from(RegisterMap::current(pid)?);
    // try to use registers for in focus frame
    debugee.restore_registers_at_frame(ctx.pid_on_focus(), &mut registers, ctx.frame_num())?;
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
