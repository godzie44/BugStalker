use crate::debugger::address::RelocatedAddress;
use crate::debugger::debugee::dwarf::EndianArcSlice;
use crate::debugger::debugee::dwarf::eval::{AddressKind, ExpressionEvaluator};
use crate::debugger::debugee::{Debugee, Location};
use crate::debugger::error::Error;
use crate::debugger::error::Error::{
    TypeBinaryRepr, UnitNotFound, UnwindNoContext, UnwindTooDeepFrame,
};
use crate::debugger::register::{DwarfRegisterMap, RegisterMap};
use crate::debugger::utils::TryGetOrInsert;
use crate::debugger::{ExplorationContext, PlaceDescriptorOwned};
use crate::{debugger, resolve_unit_call, weak_error};
use gimli::{EhFrame, FrameDescriptionEntry, RegisterRule, UnwindSection};
use nix::unistd::Pid;
use std::mem;

/// Unique frame identifier. It is just an address of the first instruction in function.
pub type FrameID = RelocatedAddress;

/// Represents detailed information about single stack frame in the unwind path.
#[derive(Debug, Default, Clone)]
pub struct FrameSpan {
    pub func_name: Option<String>,
    pub fn_start_ip: Option<RelocatedAddress>,
    pub ip: RelocatedAddress,
    pub place: Option<PlaceDescriptorOwned>,
}

impl FrameSpan {
    fn new(
        debugee: &Debugee,
        ip: RelocatedAddress,
        fn_name: Option<String>,
        fn_start_ip: Option<RelocatedAddress>,
    ) -> Result<Self, Error> {
        let debug_information = debugee.debug_info(ip)?;
        let place = debug_information
            .find_place_from_pc(ip.into_global(debugee)?)?
            .map(|p| p.to_owned());

        Ok(FrameSpan {
            func_name: fn_name,
            fn_start_ip,
            ip,
            place,
        })
    }

    #[inline(always)]
    pub fn id(&self) -> Option<FrameID> {
        self.fn_start_ip
    }
}

pub type Backtrace = Vec<FrameSpan>;

/// Unwind thread stack and return a backtrace.
///
/// # Arguments
///
/// * `debugee`: debugee instance
/// * `pid`: thread for unwinding
pub fn unwind(debugee: &Debugee, pid: Pid) -> Result<Backtrace, Error> {
    #[cfg(not(feature = "libunwind"))]
    {
        let unwinder = DwarfUnwinder::new(debugee);
        unwinder.unwind(pid)
    }
    #[cfg(feature = "libunwind")]
    libunwind::unwind(debugee, pid)
}

/// Restore registers at chosen frame.
///
/// # Arguments
///
/// * `debugee`: debugee instance
/// * `pid`: thread for unwinding
/// * `registers`: initial registers state at frame 0 (current frame), will be updated with new values
/// * `frame_num`: frame number for which registers is restored
#[allow(unused)]
pub fn restore_registers_at_frame(
    debugee: &Debugee,
    pid: Pid,
    registers: &mut DwarfRegisterMap,
    frame_num: u32,
) -> Result<(), Error> {
    #[cfg(not(feature = "libunwind"))]
    {
        let unwinder = DwarfUnwinder::new(debugee);
        unwinder.restore_registers_at_frame(pid, registers, frame_num)
    }
    #[cfg(feature = "libunwind")]
    libunwind::restore_registers_at_frame(pid, registers, frame_num)
}

/// Return return address for thread current program counter.
///
/// # Arguments
///
/// * `debugee`: debugee instance
/// * `pid`: thread for unwinding
#[allow(unused)]
pub fn return_addr(debugee: &Debugee, pid: Pid) -> Result<Option<RelocatedAddress>, Error> {
    #[cfg(not(feature = "libunwind"))]
    {
        let unwinder = DwarfUnwinder::new(debugee);
        unwinder.return_address(pid)
    }
    #[cfg(feature = "libunwind")]
    libunwind::return_addr(pid)
}

/// UnwindContext contains information for unwinding single frame.  
pub struct UnwindContext<'a> {
    registers: DwarfRegisterMap,
    location: Location,
    fde: FrameDescriptionEntry<EndianArcSlice, usize>,
    debugee: &'a Debugee,
    cfa: RelocatedAddress,
}

impl<'a> UnwindContext<'a> {
    fn new(
        debugee: &'a Debugee,
        registers: DwarfRegisterMap,
        expl_ctx: &ExplorationContext,
    ) -> Result<Option<Self>, Error> {
        let dwarf = &debugee.debug_info(expl_ctx.location().pc)?;
        let mut next_registers = registers.clone();
        let registers_snap = registers;
        let fde = match dwarf.eh_frame.fde_for_address(
            &dwarf.bases,
            expl_ctx.location().global_pc.into(),
            EhFrame::cie_from_offset,
        ) {
            Ok(fde) => fde,
            Err(gimli::Error::NoUnwindInfoForAddress) => {
                return Ok(None);
            }
            Err(e) => return Err(e.into()),
        };

        let mut ctx = Box::new(gimli::UnwindContext::new());
        let row = fde.unwind_info_for_address(
            &dwarf.eh_frame,
            &dwarf.bases,
            &mut ctx,
            expl_ctx.location().global_pc.into(),
        )?;
        let cfa = dwarf.evaluate_cfa(debugee, &registers_snap, row, expl_ctx)?;

        let mut lazy_evaluator = None;
        let evaluator_init_fn = || -> Result<ExpressionEvaluator, Error> {
            let unit = dwarf
                .find_unit_by_pc(expl_ctx.location().global_pc)?
                .ok_or(UnitNotFound(expl_ctx.location().global_pc))?;

            let evaluator =
                resolve_unit_call!(&dwarf.inner, unit, evaluator, debugee, dwarf.dwarf());
            Ok(evaluator)
        };

        row.registers()
            .filter_map(|(register, rule)| {
                let value = match rule {
                    RegisterRule::Undefined => return None,
                    RegisterRule::SameValue => {
                        let register_map =
                            weak_error!(RegisterMap::current(expl_ctx.pid_on_focus()))?;
                        weak_error!(DwarfRegisterMap::from(register_map).value(*register))?
                    }
                    RegisterRule::Offset(offset) => {
                        let addr = cfa.offset(*offset as isize);

                        let bytes = weak_error!(debugger::read_memory_by_pid(
                            expl_ctx.pid_on_focus(),
                            addr.into(),
                            mem::size_of::<u64>()
                        ))?;
                        u64::from_ne_bytes(weak_error!(bytes.try_into().map_err(
                            |data: Vec<u8>| TypeBinaryRepr("u64", data.into_boxed_slice())
                        ))?)
                    }
                    RegisterRule::ValOffset(offset) => cfa.offset(*offset as isize).into(),
                    RegisterRule::Register(reg) => weak_error!(registers_snap.value(*reg))?,
                    RegisterRule::Expression(expr) => {
                        let evaluator =
                            weak_error!(lazy_evaluator.try_get_or_insert_with(evaluator_init_fn))?;
                        let expr_result = weak_error!(evaluator.evaluate(expl_ctx, expr.clone()))?;
                        let addr = weak_error!(
                            expr_result.into_scalar::<usize>(AddressKind::MemoryAddress)
                        )?;
                        let bytes = weak_error!(debugger::read_memory_by_pid(
                            expl_ctx.pid_on_focus(),
                            addr,
                            mem::size_of::<u64>()
                        ))?;
                        u64::from_ne_bytes(weak_error!(bytes.try_into().map_err(
                            |data: Vec<u8>| TypeBinaryRepr("u64", data.into_boxed_slice())
                        ))?)
                    }
                    RegisterRule::ValExpression(expr) => {
                        let evaluator =
                            weak_error!(lazy_evaluator.try_get_or_insert_with(evaluator_init_fn))?;
                        let expr_result = weak_error!(evaluator.evaluate(expl_ctx, expr.clone()))?;
                        weak_error!(expr_result.into_scalar::<u64>(AddressKind::MemoryAddress))?
                    }
                    RegisterRule::Architectural => return None,
                    RegisterRule::Constant(val) => *val,
                    _ => unreachable!(),
                };

                Some((*register, value))
            })
            .for_each(|(reg, val)| next_registers.update(reg, val));

        Ok(Some(Self {
            registers: next_registers,
            location: expl_ctx.location(),
            debugee,
            fde,
            cfa,
        }))
    }

    pub fn next(
        previous_ctx: UnwindContext<'a>,
        ctx: &ExplorationContext,
    ) -> Result<Option<Self>, Error> {
        let mut next_frame_registers: DwarfRegisterMap = previous_ctx.registers;
        next_frame_registers.update(gimli::Register(7), previous_ctx.cfa.into());
        UnwindContext::new(previous_ctx.debugee, next_frame_registers, ctx)
    }

    fn return_address(&self) -> Option<RelocatedAddress> {
        let register = self.fde.cie().return_address_register();
        self.registers
            .value(register)
            .map(RelocatedAddress::from)
            .ok()
    }

    pub fn registers(&self) -> DwarfRegisterMap {
        self.registers.clone()
    }
}

/// Unwind debugee call stack by dwarf information.
///
/// [`DwarfUnwinder`] also useful for getting return address for current location and register values for subroutine entry.
pub struct DwarfUnwinder<'a> {
    debugee: &'a Debugee,
}

impl<'a> DwarfUnwinder<'a> {
    /// Creates new unwinder.
    ///
    /// # Arguments
    ///
    /// * `debugee`: current debugee program.
    pub fn new(debugee: &'a Debugee) -> DwarfUnwinder<'a> {
        Self { debugee }
    }

    /// Unwind call stack.
    ///
    /// # Arguments
    ///
    /// * pid: thread for unwinding
    pub fn unwind(&self, pid: Pid) -> Result<Backtrace, Error> {
        let frame_0_location = self
            .debugee
            .tracee_ctl()
            .tracee_ensure(pid)
            .location(self.debugee)?;

        let mut ctx = ExplorationContext::new(frame_0_location, 0);
        let mb_unwind_ctx = UnwindContext::new(
            self.debugee,
            DwarfRegisterMap::from(RegisterMap::current(ctx.pid_on_focus())?),
            &ctx,
        )?;
        let Some(mut unwind_ctx) = mb_unwind_ctx else {
            return Ok(vec![]);
        };

        let function = self
            .debugee
            .debug_info(ctx.location().pc)?
            .find_function_by_pc(ctx.location().global_pc)?;
        let fn_start_at = function
            .as_ref()
            .and_then(|(func, _)| {
                func.prolog_start_place().ok().map(|prolog| {
                    prolog
                        .address
                        .relocate_to_segment_by_pc(self.debugee, ctx.location().pc)
                })
            })
            .transpose()?;

        let mut bt = vec![FrameSpan::new(
            self.debugee,
            ctx.location().pc,
            function.and_then(|(_, info)| info.full_name()),
            fn_start_at,
        )?];

        // start unwind
        while let Some(return_addr) = unwind_ctx.return_address() {
            let prev_loc = bt.last().expect("backtrace len > 0");
            if prev_loc.ip == return_addr {
                break;
            }

            let next_location = Location {
                pc: return_addr,
                global_pc: return_addr.into_global(self.debugee)?,
                pid: unwind_ctx.location.pid,
            };

            ctx = ExplorationContext::new(next_location, ctx.frame_num() + 1);
            unwind_ctx = match UnwindContext::next(unwind_ctx, &ctx)? {
                None => break,
                Some(ctx) => ctx,
            };

            let function = self
                .debugee
                .debug_info(next_location.pc)?
                .find_function_by_pc(next_location.global_pc)?;
            let fn_start_at = function
                .as_ref()
                .and_then(|(die_ref, _)| {
                    die_ref.prolog_start_place().ok().map(|prolog| {
                        prolog
                            .address
                            .relocate_to_segment_by_pc(self.debugee, next_location.pc)
                    })
                })
                .transpose()?;

            let span = FrameSpan::new(
                self.debugee,
                next_location.pc,
                function.and_then(|(_, info)| info.full_name()),
                fn_start_at,
            )?;
            bt.push(span);
        }

        Ok(bt)
    }

    pub fn restore_registers_at_frame(
        &self,
        pid: Pid,
        registers: &mut DwarfRegisterMap,
        frame_num: u32,
    ) -> Result<(), Error> {
        let frame_0_location = self
            .debugee
            .tracee_ctl()
            .tracee_ensure(pid)
            .location(self.debugee)?;
        let mut ctx = ExplorationContext::new(frame_0_location, 0);

        if frame_num == 0 {
            return Ok(());
        }

        let mut unwind_ctx = UnwindContext::new(
            self.debugee,
            DwarfRegisterMap::from(RegisterMap::current(ctx.pid_on_focus())?),
            &ctx,
        )?
        .ok_or(UnwindNoContext)?;

        for _ in 0..frame_num {
            let ret_addr = unwind_ctx.return_address().ok_or(UnwindTooDeepFrame)?;

            ctx = ExplorationContext::new(
                Location {
                    pc: ret_addr,
                    global_pc: ret_addr.into_global(self.debugee)?,
                    pid: ctx.pid_on_focus(),
                },
                ctx.frame_num() + 1,
            );

            unwind_ctx = UnwindContext::next(unwind_ctx, &ctx)?.ok_or(UnwindNoContext)?;
        }

        if let Ok(ip) = unwind_ctx.registers().value(gimli::Register(16)) {
            registers.update(gimli::Register(16), ip);
        }
        if let Ok(sp) = unwind_ctx.registers().value(gimli::Register(7)) {
            registers.update(gimli::Register(7), sp);
        }

        Ok(())
    }

    /// Returns return address for stopped thread.
    ///
    /// # Arguments
    ///
    /// * `pid`: pid of stopped thread.
    pub fn return_address(&self, pid: Pid) -> Result<Option<RelocatedAddress>, Error> {
        let frame_0_location = self
            .debugee
            .tracee_ctl()
            .tracee_ensure(pid)
            .location(self.debugee)?;
        let ctx = ExplorationContext::new(frame_0_location, 0);

        let mb_unwind_ctx = UnwindContext::new(
            self.debugee,
            DwarfRegisterMap::from(RegisterMap::current(ctx.pid_on_focus())?),
            &ctx,
        )?;

        if let Some(unwind_ctx) = mb_unwind_ctx {
            return Ok(unwind_ctx.return_address());
        }
        Ok(None)
    }

    /// Returns unwind context for location.
    ///
    /// # Arguments
    ///
    /// * `location`: some debugee thread position.
    pub fn context_for(
        &self,
        ctx: &ExplorationContext,
    ) -> Result<Option<UnwindContext<'_>>, Error> {
        UnwindContext::new(
            self.debugee,
            DwarfRegisterMap::from(RegisterMap::current(ctx.pid_on_focus())?),
            ctx,
        )
    }
}

#[cfg(feature = "libunwind")]
#[deprecated(
    since = "0.3.4",
    note = "libunwind deprecated, use new unwinder `DwarfUnwinder`"
)]
mod libunwind {
    use crate::debugger::address::RelocatedAddress;
    use crate::debugger::debugee::Debugee;
    use crate::debugger::error::Error;
    use crate::debugger::register::DwarfRegisterMap;
    use crate::debugger::unwind::{Backtrace, FrameSpan};
    use nix::unistd::Pid;
    use unwind::{Accessors, AddressSpace, Byteorder, Cursor, PTraceState, RegNum};

    /// Unwind thread stack and returns backtrace.
    ///
    /// # Arguments
    ///
    /// * `pid`: thread for unwinding.
    pub(super) fn unwind(debugee: &Debugee, pid: Pid) -> Result<Backtrace, Error> {
        let state = PTraceState::new(pid.as_raw() as u32)?;
        let address_space = AddressSpace::new(Accessors::ptrace(), Byteorder::DEFAULT)?;
        let mut cursor = Cursor::remote(&address_space, &state)?;
        let mut backtrace = vec![];

        loop {
            let ip = cursor.register(RegNum::IP)?;
            match (cursor.procedure_info(), cursor.procedure_name()) {
                (Ok(ref info), Ok(ref name)) if ip == info.start_ip() + name.offset() => {
                    let fn_name = format!("{:#}", rustc_demangle::demangle(name.name()));

                    backtrace.push(FrameSpan::new(
                        debugee,
                        ip.into(),
                        Some(fn_name),
                        Some(info.start_ip().into()),
                    )?);
                }
                _ => {
                    backtrace.push(FrameSpan::new(debugee, ip.into(), None, None)?);
                }
            }

            if !cursor.step()? {
                break;
            }
        }

        Ok(backtrace)
    }

    /// Returns return address for stopped thread.
    ///
    /// # Arguments
    ///
    /// * `pid`: pid of stopped thread.
    pub(super) fn return_addr(pid: Pid) -> Result<Option<RelocatedAddress>, Error> {
        let state = PTraceState::new(pid.as_raw() as u32)?;
        let address_space = AddressSpace::new(Accessors::ptrace(), Byteorder::DEFAULT)?;
        let mut cursor = Cursor::remote(&address_space, &state)?;

        if !cursor.step()? {
            return Ok(None);
        }

        Ok(Some(RelocatedAddress::from(cursor.register(RegNum::IP)?)))
    }

    pub(super) fn restore_registers_at_frame(
        pid: Pid,
        registers: &mut DwarfRegisterMap,
        frame_num: u32,
    ) -> Result<(), Error> {
        if frame_num == 0 {
            return Ok(());
        }

        let state = PTraceState::new(pid.as_raw() as u32)?;
        let address_space = AddressSpace::new(Accessors::ptrace(), Byteorder::DEFAULT)?;
        let mut cursor = Cursor::remote(&address_space, &state)?;

        for _ in 0..frame_num {
            if !cursor.step()? {
                return Ok(());
            }
        }

        if let Ok(ip) = cursor.register(RegNum::IP) {
            registers.update(gimli::Register(16), ip);
        }
        if let Ok(sp) = cursor.register(RegNum::SP) {
            registers.update(gimli::Register(7), sp);
        }

        Ok(())
    }
}
