use crate::debugger::address::RelocatedAddress;
use crate::debugger::debugee::dwarf::EndianArcSlice;
use crate::debugger::debugee::dwarf::eval::{AddressKind, ExpressionEvaluator};
use crate::debugger::debugee::{Debugee, Location};
use crate::debugger::error::Error;
use crate::debugger::error::Error::{
    TypeBinaryRepr, UnitNotFound, UnwindNoContext, UnwindTooDeepFrame,
};
use crate::debugger::register::{DwarfRegisterMap, Register, RegisterMap};
use crate::debugger::utils::TryGetOrInsert;
use crate::debugger::{ExplorationContext, PlaceDescriptorOwned};
use crate::{debugger, resolve_unit_call, weak_error};
use gimli::{DebugFrame, EhFrame, FrameDescriptionEntry, RegisterRule, UnwindSection};
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
    let unwinder = DwarfUnwinder::new(debugee);
    unwinder.unwind(pid)
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
    let unwinder = DwarfUnwinder::new(debugee);
    unwinder.restore_registers_at_frame(pid, registers, frame_num)
}

/// Return return address for thread current program counter.
///
/// # Arguments
///
/// * `debugee`: debugee instance
/// * `pid`: thread for unwinding
#[allow(unused)]
pub fn return_addr(debugee: &Debugee, pid: Pid) -> Result<Option<RelocatedAddress>, Error> {
    let unwinder = DwarfUnwinder::new(debugee);
    unwinder.return_address(pid)
}

/// UnwindContext (or ucx) contains information for unwinding single frame.  
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
        ecx: &ExplorationContext,
    ) -> Result<Option<Self>, Error> {
        let dwarf = &debugee.debug_info(ecx.location().pc)?;
        let mut next_registers = registers.clone();
        let registers_snap = registers;
        let mut ucx = Box::new(gimli::UnwindContext::new());
        let (fde, row) = match dwarf.eh_frame.fde_for_address(
            &dwarf.bases,
            ecx.location().global_pc.into(),
            EhFrame::cie_from_offset,
        ) {
            Ok(fde) => {
                let row = fde.unwind_info_for_address(
                    &dwarf.eh_frame,
                    &dwarf.bases,
                    &mut ucx,
                    ecx.location().global_pc.into(),
                )?;
                (fde, row)
            }
            Err(gimli::Error::NoUnwindInfoForAddress) => {
                let Some(debug_frame) = dwarf.debug_frame.as_ref() else {
                    return Ok(None);
                };
                let fde = match debug_frame.fde_for_address(
                    &dwarf.bases,
                    ecx.location().global_pc.into(),
                    DebugFrame::cie_from_offset,
                ) {
                    Ok(fde) => fde,
                    Err(gimli::Error::NoUnwindInfoForAddress) => return Ok(None),
                    Err(e) => return Err(e.into()),
                };
                let row = fde.unwind_info_for_address(
                    debug_frame,
                    &dwarf.bases,
                    &mut ucx,
                    ecx.location().global_pc.into(),
                )?;
                (fde, row)
            }
            Err(e) => return Err(e.into()),
        };
        let cfa = dwarf.evaluate_cfa(debugee, &registers_snap, row, ecx)?;

        let mut lazy_evaluator = None;
        let evaluator_init_fn = || -> Result<ExpressionEvaluator, Error> {
            let unit = dwarf
                .find_unit_by_pc(ecx.location().global_pc)?
                .ok_or(UnitNotFound(ecx.location().global_pc))?;

            let evaluator =
                resolve_unit_call!(&dwarf.inner, unit, evaluator, debugee, dwarf.dwarf());
            Ok(evaluator)
        };

        let read_register_value = |addr: RelocatedAddress| -> Option<u64> {
            let bytes = weak_error!(debugger::read_memory_by_pid(
                ecx.pid_on_focus(),
                addr.into(),
                mem::size_of::<usize>()
            ))?;
            let value = usize::from_ne_bytes(weak_error!(
                bytes
                    .try_into()
                    .map_err(|data: Vec<u8>| TypeBinaryRepr("usize", data.into_boxed_slice()))
            )?);
            Some(value as u64)
        };

        row.registers()
            .filter_map(|(register, rule)| {
                let value = match rule {
                    RegisterRule::Undefined => return None,
                    RegisterRule::SameValue => weak_error!(registers_snap.value(*register))?,
                    RegisterRule::Offset(offset) => {
                        let addr = cfa.offset(*offset as isize);
                        read_register_value(addr)?
                    }
                    RegisterRule::ValOffset(offset) => cfa.offset(*offset as isize).into(),
                    RegisterRule::Register(reg) => weak_error!(registers_snap.value(*reg))?,
                    RegisterRule::Expression(expr) => {
                        let evaluator =
                            weak_error!(lazy_evaluator.try_get_or_insert_with(evaluator_init_fn))?;
                        let expr_result = weak_error!(evaluator.evaluate(ecx, expr.clone()))?;
                        let addr = weak_error!(
                            expr_result.into_scalar::<usize>(AddressKind::MemoryAddress)
                        )?;
                        read_register_value(RelocatedAddress::from(addr))?
                    }
                    RegisterRule::ValExpression(expr) => {
                        let evaluator =
                            weak_error!(lazy_evaluator.try_get_or_insert_with(evaluator_init_fn))?;
                        let expr_result = weak_error!(evaluator.evaluate(ecx, expr.clone()))?;
                        weak_error!(expr_result.into_scalar::<usize>(AddressKind::MemoryAddress))?
                            as u64
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
            location: ecx.location(),
            debugee,
            fde,
            cfa,
        }))
    }

    pub fn next(
        previous_ucx: UnwindContext<'a>,
        ecx: &ExplorationContext,
    ) -> Result<Option<Self>, Error> {
        let mut next_frame_registers: DwarfRegisterMap = previous_ucx.registers;
        let sp_register = Register::Rsp
            .dwarf_register()
            .expect("stack pointer register must map to dwarf register");
        next_frame_registers.update(sp_register, previous_ucx.cfa.into());
        UnwindContext::new(previous_ucx.debugee, next_frame_registers, ecx)
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

        let mut ecx = ExplorationContext::new(frame_0_location, 0);
        let mb_ucx = UnwindContext::new(
            self.debugee,
            DwarfRegisterMap::from(RegisterMap::current(ecx.pid_on_focus())?),
            &ecx,
        )?;

        let function = self
            .debugee
            .debug_info(ecx.location().pc)?
            .find_function_by_pc(ecx.location().global_pc)?;
        let fn_start_at = function
            .as_ref()
            .and_then(|(func, _)| {
                func.prolog_start_place().ok().map(|prolog| {
                    prolog
                        .address
                        .relocate_to_segment_by_pc(self.debugee, ecx.location().pc)
                })
            })
            .transpose()?;

        let mut bt = vec![FrameSpan::new(
            self.debugee,
            ecx.location().pc,
            function.and_then(|(_, info)| info.full_name()),
            fn_start_at,
        )?];
        let Some(mut ucx) = mb_ucx else {
            return Ok(bt);
        };

        // start unwind
        while let Some(return_addr) = ucx.return_address() {
            let prev_loc = bt.last().expect("backtrace len > 0");
            if prev_loc.ip == return_addr {
                break;
            }

            let next_location = Location {
                pc: return_addr,
                global_pc: return_addr.into_global(self.debugee)?,
                pid: ucx.location.pid,
            };

            ecx = ExplorationContext::new(next_location, ecx.frame_num() + 1);
            ucx = match UnwindContext::next(ucx, &ecx)? {
                None => break,
                Some(ucx) => ucx,
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
        let mut ecx = ExplorationContext::new(frame_0_location, 0);

        if frame_num == 0 {
            return Ok(());
        }

        let mut unwind_ucx = UnwindContext::new(
            self.debugee,
            DwarfRegisterMap::from(RegisterMap::current(ecx.pid_on_focus())?),
            &ecx,
        )?
        .ok_or(UnwindNoContext)?;

        for _ in 0..frame_num {
            let ret_addr = unwind_ucx.return_address().ok_or(UnwindTooDeepFrame)?;

            ecx = ExplorationContext::new(
                Location {
                    pc: ret_addr,
                    global_pc: ret_addr.into_global(self.debugee)?,
                    pid: ecx.pid_on_focus(),
                },
                ecx.frame_num() + 1,
            );

            unwind_ucx = UnwindContext::next(unwind_ucx, &ecx)?.ok_or(UnwindNoContext)?;
        }

        let unwind_registers = unwind_ucx.registers();
        registers.update_from(&unwind_registers);

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
        let ecx = ExplorationContext::new(frame_0_location, 0);

        let mb_ucx = UnwindContext::new(
            self.debugee,
            DwarfRegisterMap::from(RegisterMap::current(ecx.pid_on_focus())?),
            &ecx,
        )?;

        if let Some(ucx) = mb_ucx {
            return Ok(ucx.return_address());
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
        ecx: &ExplorationContext,
    ) -> Result<Option<UnwindContext<'_>>, Error> {
        UnwindContext::new(
            self.debugee,
            DwarfRegisterMap::from(RegisterMap::current(ecx.pid_on_focus())?),
            ecx,
        )
    }
}
