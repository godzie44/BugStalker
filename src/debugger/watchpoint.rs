use crate::debugger::Error::Hook;
use crate::debugger::address::{GlobalAddress, RelocatedAddress};
use crate::debugger::breakpoint::{Breakpoint, BreakpointRegistry};
use crate::debugger::debugee::dwarf::r#type::TypeIdentity;
use crate::debugger::debugee::tracee::TraceeCtl;
use crate::debugger::debugee::tracer::WatchpointHitType;
use crate::debugger::debugee::{Debugee, Location};
use crate::debugger::register::debug::{
    BreakCondition, BreakSize, DebugRegisterNumber, HardwareDebugState,
};
use crate::debugger::unwind::FrameID;
use crate::debugger::variable::dqe::Dqe;
use crate::debugger::variable::execute::{DqeExecutor, QueryResult};
use crate::debugger::variable::value::{ScalarValue, SupportedScalar, Value};
use crate::debugger::{Debugger, Error, ExplorationContext, Tracee};
use crate::{debugger, disable_when_not_stared, weak_error};
use log::error;
use nix::unistd::Pid;
use std::borrow::Cow;
use std::mem;
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Debug)]
struct ExpressionTarget {
    /// Original DQE string.
    source_string: String,
    /// Address DQE.
    dqe: Dqe,
    /// Last evaluated underlying DQE result.
    last_value: Option<Value>,
    /// ID of in-focus frame at the time when watchpoint was created.
    /// Whether `None` when underlying expression has a global or undefined scope.
    frame_id: Option<FrameID>,
    /// ID of in-focus thread at the time when watchpoint was created.
    tid: Pid,
    /// Contains breakpoint number if watchpoint DQE is scoped (have a limited lifetime).
    /// This breakpoint points to the end of DQE scope.
    companion: Option<u32>,
}

impl ExpressionTarget {
    fn underlying_dqe(&self) -> &Dqe {
        let Dqe::Address(ref underlying_dqe) = self.dqe else {
            unreachable!("infallible: watchpoint always contains an address DQE");
        };
        underlying_dqe
    }
}

#[derive(Debug)]
struct AddressTarget {
    /// Last seen dereferenced value.
    /// This is [`Value::Scalar`] with one of u8, u16, u32 or u64 underlying value.
    last_value: Option<Value>,
}

impl AddressTarget {
    fn refresh_last_value(&mut self, pid: Pid, hw: &HardwareBreakpoint) -> Option<Value> {
        let read_size = match hw.size {
            BreakSize::Bytes1 => 1,
            BreakSize::Bytes2 => 2,
            BreakSize::Bytes8 => 8,
            BreakSize::Bytes4 => 4,
        };

        let maybe_data = weak_error!(debugger::read_memory_by_pid(
            pid,
            hw.address.as_usize(),
            read_size
        ));
        let new_val = maybe_data.map(|data| {
            let (t, u) = match hw.size {
                BreakSize::Bytes1 => (
                    "u8",
                    SupportedScalar::U8(u8::from_ne_bytes(
                        data.try_into().expect("unexpected size"),
                    )),
                ),
                BreakSize::Bytes2 => (
                    "u16",
                    SupportedScalar::U16(u16::from_ne_bytes(
                        data.try_into().expect("unexpected size"),
                    )),
                ),
                BreakSize::Bytes8 => (
                    "u64",
                    SupportedScalar::U64(u64::from_ne_bytes(
                        data.try_into().expect("unexpected size"),
                    )),
                ),
                BreakSize::Bytes4 => (
                    "u32",
                    SupportedScalar::U32(u32::from_ne_bytes(
                        data.try_into().expect("unexpected size"),
                    )),
                ),
            };
            Value::Scalar(ScalarValue {
                value: Some(u),
                type_ident: TypeIdentity::no_namespace(t),
                type_id: None,
                raw_address: None,
            })
        });

        mem::replace(&mut self.last_value, new_val)
    }
}

#[derive(Debug)]
enum Subject {
    /// Watchpoint with DQE result as an observed subject.
    Expression(ExpressionTarget),
    /// Watchpoint with a memory location as an observed subject.
    Address(AddressTarget),
}

#[derive(Debug)]
struct HardwareBreakpoint {
    /// Address in debugee memory where hardware breakpoint is set.
    address: RelocatedAddress,
    /// Size of watch location at the address.
    size: BreakSize,
    /// Hardware register.
    register: Option<DebugRegisterNumber>,
    /// Associated condition.
    condition: BreakCondition,
}

impl HardwareBreakpoint {
    fn new(address: RelocatedAddress, size: BreakSize, condition: BreakCondition) -> Self {
        Self {
            address,
            size,
            register: None,
            condition,
        }
    }

    fn enable(&mut self, tracee_ctl: &TraceeCtl) -> Result<HardwareDebugState, Error> {
        let mut state = HardwareDebugState::current(tracee_ctl.proc_pid())?;

        // trying to find free debug register
        let free_register = [
            DebugRegisterNumber::DR0,
            DebugRegisterNumber::DR1,
            DebugRegisterNumber::DR2,
            DebugRegisterNumber::DR3,
        ]
        .into_iter()
        .find(|&dr_num| !state.dr7.dr_enabled(dr_num, false))
        .ok_or(Error::WatchpointLimitReached)?;

        // set hardware breakpoint
        state.address_regs[free_register as usize] = self.address.as_usize();
        state
            .dr7
            .configure_bp(free_register, self.condition, self.size);
        state.dr7.set_dr(free_register, false, true);
        tracee_ctl.tracee_iter().for_each(|t| {
            if let Err(e) = state.sync(t.pid) {
                error!("set hardware breakpoint for thread {}: {e}", t.pid)
            }
        });
        self.register = Some(free_register);

        Ok(state)
    }

    fn disable(&mut self, tracee_ctl: &TraceeCtl) -> Result<HardwareDebugState, Error> {
        let mut state = HardwareDebugState::current(tracee_ctl.proc_pid())?;
        let register = self.register.expect("should exist");
        state.dr7.set_dr(register, false, false);
        tracee_ctl.tracee_iter().for_each(|t| {
            if let Err(e) = state.sync(t.pid) {
                error!("remove hardware breakpoint for thread {}: {e}", t.pid)
            }
        });
        self.register = None;
        Ok(state)
    }

    fn address_already_observed(
        tracee_ctl: &TraceeCtl,
        address: RelocatedAddress,
    ) -> Result<bool, Error> {
        let state = HardwareDebugState::current(tracee_ctl.proc_pid())?;
        Ok(state
            .address_regs
            .iter()
            .enumerate()
            .any(|(dr, in_use_addr)| {
                let enabled = state.dr7.dr_enabled(
                    DebugRegisterNumber::from_repr(dr).expect("infallible"),
                    false,
                );
                enabled && *in_use_addr == address.as_usize()
            }))
    }
}

static GLOBAL_WP_COUNTER: AtomicU32 = AtomicU32::new(1);

/// Watchpoint representation.
#[derive(Debug)]
pub struct Watchpoint {
    /// Watchpoint number, started from 1.
    number: u32,
    /// Underlying hardware breakpoint.
    hw: HardwareBreakpoint,
    /// Subject for observation.
    subject: Subject,
    /// Temporary watchpoint may be created by step's algorithms
    temporary: bool,
}

fn call_with_context<F, T>(debugger: &mut Debugger, ecx: ExplorationContext, f: F) -> T
where
    F: FnOnce(&Debugger) -> T,
{
    let old_ecx = mem::replace(&mut debugger.expl_context, ecx);
    let result = f(debugger);
    debugger.expl_context = old_ecx;
    result
}

impl Watchpoint {
    pub fn is_temporary(&self) -> bool {
        self.temporary
    }

    fn execute_dqe(debugger: &Debugger, dqe: Dqe) -> Result<QueryResult<'_>, Error> {
        let executor = DqeExecutor::new(debugger);

        // trying to evaluate at variables first,
        // if a result is empty, try to evaluate at function arguments
        let mut evaluation_on_vars_results = executor.query(&dqe)?;
        let mut evaluation_on_args_results;
        let expr_result = match evaluation_on_vars_results.len() {
            0 => {
                evaluation_on_args_results = executor.query_arguments(&dqe)?;
                match evaluation_on_args_results.len() {
                    0 => return Err(Error::WatchSubjectNotFound),
                    1 => evaluation_on_args_results.pop().expect("infallible"),
                    _ => return Err(Error::WatchpointCollision),
                }
            }
            1 => evaluation_on_vars_results.pop().expect("infallible"),
            _ => return Err(Error::WatchpointCollision),
        };
        Ok(expr_result)
    }

    /// Create watchpoint using a result of DQE as a subject for observation.
    ///
    /// # Arguments
    ///
    /// * `debugger`: debugger instance
    /// * `expr_source`: DQE string representation
    /// * `dqe`: DQE
    /// * `condition`: condition for activating a watchpoint
    pub fn from_dqe(
        debugger: &mut Debugger,
        expr_source: &str,
        dqe: Dqe,
        condition: BreakCondition,
    ) -> Result<(HardwareDebugState, Self), Error> {
        // wrap expression with address operation
        let dqe = Dqe::Address(dqe.boxed());

        let address_dqe_result = Self::execute_dqe(debugger, dqe.clone())?;
        let Value::Pointer(ptr) = address_dqe_result.value() else {
            unreachable!("infallible: address DQE always return a pointer")
        };

        let address = RelocatedAddress::from(ptr.value.ok_or(Error::WatchpointNoAddress)? as usize);
        if HardwareBreakpoint::address_already_observed(debugger.debugee.tracee_ctl(), address)? {
            return Err(Error::AddressAlreadyObserved);
        }

        let size = ptr.target_type_size.ok_or(Error::WatchpointUndefinedSize)?;
        if size > u8::MAX as u64 {
            return Err(Error::WatchpointWrongSize);
        };
        let size = BreakSize::try_from(size as u8)?;

        let mut end_of_scope_brkpt = None;
        let mut frame_id = None;
        if let Some(scope) = address_dqe_result.scope() {
            // take a current frame id
            let ecx = debugger.ecx();
            let frame_num = ecx.frame_num();
            let backtrace = debugger.backtrace(ecx.pid_on_focus())?;
            frame_id = backtrace
                .get(frame_num as usize)
                .ok_or(Error::FrameNotFound(frame_num))?
                .id();

            let pc = ecx.location().pc;
            let dwarf = debugger.debugee.debug_info(pc)?;

            // from all expression ranges take end-address with maximum line number -
            // this will be an address of a companion breakpoint
            let mut best_place = None;

            for range in scope.iter() {
                let maybe_place = dwarf.find_exact_place_from_pc(GlobalAddress::from(range.end))?;
                if let Some(range_end_place) = maybe_place {
                    let mut place = range_end_place.clone();

                    // find a suitable place algorithm:
                    // 1) DOWN phase: for each next place from range_end_place:
                    // 1.1) if place is a statement - suitable place found
                    // 1.2) if no next place, or next place is ET or EB - go to UP phase
                    // 2) UP phase: for each previous place from range_end_place:
                    // 2.1) if place is a statement - suitable place found
                    // 2.2) if place address <= range.begin - suitable place not found
                    // 2.3) if no next place, or previous place is ET or PE - suitable place not found

                    let mut suitable_place = loop {
                        if place.is_stmt {
                            break Some(place);
                        }
                        if place.epilog_begin || place.end_sequence {
                            break None;
                        }
                        match place.next() {
                            None => break None,
                            Some(p) => place = p,
                        }
                    };

                    if suitable_place.is_none()
                        && let Some(mut place) = range_end_place.prev()
                    {
                        suitable_place = loop {
                            if place.address <= GlobalAddress::from(range.begin) {
                                break None;
                            }
                            if place.is_stmt {
                                break Some(place);
                            }
                            if place.prolog_end || place.end_sequence {
                                break None;
                            }
                            match place.prev() {
                                None => break None,
                                Some(p) => place = p,
                            }
                        };
                    }

                    if let Some(suitable_place) = suitable_place {
                        match best_place {
                            None => best_place = Some(suitable_place),
                            Some(max) if max.line_number <= suitable_place.line_number => {
                                best_place = Some(suitable_place)
                            }
                            _ => {}
                        }
                    }
                }
            }

            let best_place = best_place.ok_or(Error::UnknownScope)?;

            let end_of_scope = best_place
                .address
                .relocate_to_segment(&debugger.debugee, dwarf)?;
            let next_wp_num = GLOBAL_WP_COUNTER.load(Ordering::Relaxed);
            let brkpt = Breakpoint::new_watchpoint_companion(
                &debugger.breakpoints,
                next_wp_num,
                end_of_scope,
                ecx.pid_on_focus(),
            );
            let brkpt_view = debugger.breakpoints.add_and_enable(brkpt)?;
            end_of_scope_brkpt = Some(brkpt_view.number);
        }

        let mut target = ExpressionTarget {
            source_string: expr_source.to_string(),
            dqe,
            last_value: None,
            frame_id,
            tid: debugger.ecx().pid_on_focus(),
            companion: end_of_scope_brkpt,
        };
        let underlying_dqe = target.underlying_dqe().clone();
        let var = Self::execute_dqe(debugger, underlying_dqe)
            .map(|ev| ev.into_value())
            .ok();
        target.last_value = var;

        let mut hw_brkpt = HardwareBreakpoint::new(address, size, condition);
        let state = hw_brkpt.enable(debugger.debugee.tracee_ctl())?;

        let this = Self {
            number: GLOBAL_WP_COUNTER.fetch_add(1, Ordering::Relaxed),
            hw: hw_brkpt,
            subject: Subject::Expression(target),
            temporary: false,
        };
        Ok((state, this))
    }

    /// Create watchpoint using a memory location address and size.
    ///
    /// # Arguments
    ///
    /// * `tracee_ctl`: a source of information about tracee's
    /// * `addr`: memory location address
    /// * `size`: memory location size
    /// * `condition`: condition for activating a watchpoint
    /// * `temporary`: set temporary flag, temporary watchpoint ignores in hooks
    fn from_raw_addr(
        tracee_ctl: &TraceeCtl,
        addr: RelocatedAddress,
        size: BreakSize,
        condition: BreakCondition,
        temporary: bool,
    ) -> Result<(HardwareDebugState, Self), Error> {
        debug_assert!(
            condition == BreakCondition::DataWrites || condition == BreakCondition::DataReadsWrites
        );
        if HardwareBreakpoint::address_already_observed(tracee_ctl, addr)? {
            return Err(Error::AddressAlreadyObserved);
        }
        let mut hw_brkpt = HardwareBreakpoint::new(addr, size, condition);
        let state = hw_brkpt.enable(tracee_ctl)?;

        let mut target = AddressTarget { last_value: None };
        target.refresh_last_value(tracee_ctl.proc_pid(), &hw_brkpt);

        let this = Self {
            number: GLOBAL_WP_COUNTER.fetch_add(1, Ordering::Relaxed),
            hw: hw_brkpt,
            subject: Subject::Address(target),
            temporary,
        };

        Ok((state, this))
    }

    /// Return underlying hardware register.
    pub fn register(&self) -> Option<DebugRegisterNumber> {
        self.hw.register
    }

    /// Return watchpoint number.
    pub fn number(&self) -> u32 {
        self.number
    }

    fn last_value(&self) -> Option<&Value> {
        match &self.subject {
            Subject::Expression(e) => e.last_value.as_ref(),
            Subject::Address(_) => None,
        }
    }

    fn scoped(&self) -> bool {
        matches!(
            self.subject,
            Subject::Expression(ExpressionTarget {
                companion: Some(_),
                ..
            })
        )
    }

    /// Disable watchpoint at all tracees, return new hardware debug registers state.
    ///
    /// # Arguments
    ///
    /// * `tracee_ctl`: a source of information about tracees
    /// * `breakpoints`: breakpoint registry, used for a remove a companion
    pub fn disable(
        &mut self,
        tracee_ctl: &TraceeCtl,
        breakpoints: &mut BreakpointRegistry,
    ) -> Result<HardwareDebugState, Error> {
        // disable hardware breakpoint
        let state = self.hw.disable(tracee_ctl)?;

        if let Subject::Expression(ref e) = self.subject {
            // decrease companion breakpoint refcount
            if let Some(brkpt) = e.companion {
                breakpoints.decrease_companion_rc(brkpt, self.number)?;
            }
        }
        Ok(state)
    }

    /// Enable watchpoint from disabled state.
    fn refresh(&mut self, tracee_ctl: &TraceeCtl) -> Result<HardwareDebugState, Error> {
        if let Subject::Expression(ref mut expr_t) = self.subject {
            expr_t.last_value = None;
        }
        let state = self.hw.enable(tracee_ctl)?;
        Ok(state)
    }
}

/// Watchpoint information struct.
pub struct WatchpointView<'a> {
    pub number: u32,
    pub address: RelocatedAddress,
    pub condition: BreakCondition,
    pub source_dqe: Option<Cow<'a, str>>,
    pub size: BreakSize,
}

impl<'a> From<&'a Watchpoint> for WatchpointView<'a> {
    fn from(wp: &'a Watchpoint) -> Self {
        Self {
            number: wp.number,
            address: wp.hw.address,
            condition: wp.hw.condition,
            source_dqe: if let Subject::Expression(ref t) = wp.subject {
                Some(Cow::Borrowed(&t.source_string))
            } else {
                None
            },
            size: wp.hw.size,
        }
    }
}

impl From<Watchpoint> for WatchpointView<'_> {
    fn from(mut wp: Watchpoint) -> Self {
        Self {
            number: wp.number,
            address: wp.hw.address,
            condition: wp.hw.condition,
            source_dqe: if let Subject::Expression(ref mut t) = wp.subject {
                let s = mem::take(&mut t.source_string);
                Some(Cow::Owned(s))
            } else {
                None
            },
            size: wp.hw.size,
        }
    }
}

impl WatchpointView<'_> {
    pub fn to_owned(&self) -> WatchpointViewOwned {
        WatchpointViewOwned {
            number: self.number,
            address: self.address,
            condition: self.condition,
            source_dqe: self.source_dqe.as_ref().map(|dqe| dqe.to_string()),
            size: self.size,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WatchpointViewOwned {
    pub number: u32,
    pub address: RelocatedAddress,
    pub condition: BreakCondition,
    pub source_dqe: Option<String>,
    pub size: BreakSize,
}

/// Container for application watchpoints.
#[derive(Default)]
pub struct WatchpointRegistry {
    /// Watchpoints list.
    watchpoints: Vec<Watchpoint>,
    /// Last used state of hardware debug registers. Update at inserting and removing watchpoints
    /// from registry.
    last_seen_state: Option<HardwareDebugState>,
}

impl WatchpointRegistry {
    fn add(&mut self, state: HardwareDebugState, wp: Watchpoint) -> WatchpointView<'_> {
        self.last_seen_state = Some(state);
        self.watchpoints.push(wp);

        (&self.watchpoints[self.watchpoints.len() - 1]).into()
    }

    /// Return watchpoint by number.
    #[inline(always)]
    pub fn get(&self, number: u32) -> Option<&Watchpoint> {
        self.watchpoints.iter().find(|wp| wp.number() == number)
    }

    /// Return all watchpoints.
    #[inline(always)]
    pub fn all(&self) -> &[Watchpoint] {
        self.watchpoints.as_slice()
    }

    /// Return all watchpoints (mutable).
    #[inline(always)]
    pub fn all_mut(&mut self) -> &mut [Watchpoint] {
        self.watchpoints.as_mut()
    }

    fn remove(
        &mut self,
        tracee_ctl: &TraceeCtl,
        brkpts: &mut BreakpointRegistry,
        idx: usize,
    ) -> Result<Option<WatchpointView<'_>>, Error> {
        let mut wp = self.watchpoints.remove(idx);
        let state = wp.disable(tracee_ctl, brkpts)?;
        self.last_seen_state = Some(state);
        Ok(Some(wp.into()))
    }

    /// Remove watchpoint by number.
    fn remove_by_num(
        &mut self,
        tracee_ctl: &TraceeCtl,
        breakpoints: &mut BreakpointRegistry,
        num: u32,
    ) -> Result<Option<WatchpointView<'_>>, Error> {
        let Some(to_remove) = self.watchpoints.iter().position(|wp| wp.number == num) else {
            return Ok(None);
        };
        self.remove(tracee_ctl, breakpoints, to_remove)
    }

    /// Remove watchpoint by memory location address.
    fn remove_by_addr(
        &mut self,
        tracee_ctl: &TraceeCtl,
        breakpoints: &mut BreakpointRegistry,
        addr: RelocatedAddress,
    ) -> Result<Option<WatchpointView<'_>>, Error> {
        let Some(to_remove) = self.watchpoints.iter().position(|wp| wp.hw.address == addr) else {
            return Ok(None);
        };
        self.remove(tracee_ctl, breakpoints, to_remove)
    }

    /// Remove watchpoint by underlying DQE.
    fn remove_by_dqe(
        &mut self,
        tracee_ctl: &TraceeCtl,
        breakpoints: &mut BreakpointRegistry,
        dqe: Dqe,
    ) -> Result<Option<WatchpointView<'_>>, Error> {
        let needle = Dqe::Address(dqe.boxed());
        let Some(to_remove) = self.watchpoints.iter().position(|wp| {
            if let Subject::Expression(ExpressionTarget { dqe: wp_dqe, .. }) = &wp.subject {
                &needle == wp_dqe
            } else {
                false
            }
        }) else {
            return Ok(None);
        };
        self.remove(tracee_ctl, breakpoints, to_remove)
    }

    /// Remove all watchpoints.
    pub fn clear_all(&mut self, tracee_ctl: &TraceeCtl, breakpoints: &mut BreakpointRegistry) {
        let wp_count = self.watchpoints.len();
        for _ in 0..wp_count {
            weak_error!(self.remove(tracee_ctl, breakpoints, 0));
        }
        self.last_seen_state = None;
    }

    /// Remove all scoped watchpoints (typically it is watchpoints at local variables)
    /// and disable non-scoped.
    pub fn clear_local_disable_global(
        &mut self,
        tracee_ctl: &TraceeCtl,
        breakpoints: &mut BreakpointRegistry,
    ) -> Vec<Error> {
        let wp_count = self.watchpoints.len();
        let mut result = vec![];

        let mut j = 0;
        for _ in 0..wp_count {
            if self.watchpoints[j].scoped() {
                if let Err(e) = self.remove(tracee_ctl, breakpoints, j) {
                    result.push(e);
                }
            } else {
                if let Err(e) = self.watchpoints[j].disable(tracee_ctl, breakpoints) {
                    result.push(e);
                }
                j += 1;
            }
        }
        self.last_seen_state = None;
        result
    }

    /// Distribute all existed watchpoints to a new tracee (thread).
    pub fn distribute_to_tracee(&self, tracee: &Tracee) -> Result<(), Error> {
        if let Some(ref state) = self.last_seen_state
            && let Err(e) = state.sync(tracee.pid)
        {
            error!("set hardware breakpoint for thread {}: {e}", tracee.pid)
        }
        Ok(())
    }

    /// Enable all previously disabled watchpoints.
    pub fn refresh(&mut self, debugee: &Debugee) -> Vec<Error> {
        self.watchpoints
            .iter_mut()
            .filter_map(|wp| {
                // local breakpoints must be removed when the registry is hibernated
                debug_assert!(!wp.scoped());
                match wp.refresh(debugee.tracee_ctl()) {
                    Ok(state) => {
                        self.last_seen_state = Some(state);
                        None
                    }
                    Err(e) => Some(e),
                }
            })
            .collect()
    }
}

impl Debugger {
    /// Set a new watchpoint on a result of DQE.
    ///
    /// # Arguments
    ///
    /// * `expr_source`: expression string
    /// * `dqe`: DQE
    /// * `condition`: condition for activating a watchpoint
    pub fn set_watchpoint_on_expr(
        &mut self,
        expr_source: &str,
        dqe: Dqe,
        condition: BreakCondition,
    ) -> Result<WatchpointView<'_>, Error> {
        disable_when_not_stared!(self);
        let (hw_state, wp) = Watchpoint::from_dqe(self, expr_source, dqe, condition)?;
        Ok(self.watchpoints.add(hw_state, wp))
    }

    /// Set a new watchpoint on a memory location
    ///
    /// # Arguments
    ///
    /// * `addr`: address in debugee memory
    /// * `size`: size of debugee memory location
    /// * `condition`: condition for activating a watchpoint
    pub fn set_watchpoint_on_memory(
        &mut self,
        addr: RelocatedAddress,
        size: BreakSize,
        condition: BreakCondition,
        temporary: bool,
    ) -> Result<WatchpointView<'_>, Error> {
        disable_when_not_stared!(self);
        let (hw_state, wp) =
            Watchpoint::from_raw_addr(self.debugee.tracee_ctl(), addr, size, condition, temporary)?;
        Ok(self.watchpoints.add(hw_state, wp))
    }

    /// Remove watchpoint by its number
    ///
    /// # Arguments
    ///
    /// * `num`: watchpoint number
    pub fn remove_watchpoint_by_number(
        &mut self,
        num: u32,
    ) -> Result<Option<WatchpointView<'_>>, Error> {
        let breakpoints = &mut self.breakpoints;
        self.watchpoints
            .remove_by_num(self.debugee.tracee_ctl(), breakpoints, num)
    }

    /// Remove watchpoint by observed address in debugee memory.
    ///
    /// # Arguments
    ///
    /// * `addr`: address in debugee memory
    pub fn remove_watchpoint_by_addr(
        &mut self,
        addr: RelocatedAddress,
    ) -> Result<Option<WatchpointView<'_>>, Error> {
        let breakpoints = &mut self.breakpoints;
        self.watchpoints
            .remove_by_addr(self.debugee.tracee_ctl(), breakpoints, addr)
    }

    /// Remove watchpoint by DQE, which result observed.
    ///
    /// # Arguments
    ///
    /// * `dqe`: DQE
    pub fn remove_watchpoint_by_expr(
        &mut self,
        dqe: Dqe,
    ) -> Result<Option<WatchpointView<'_>>, Error> {
        let breakpoints = &mut self.breakpoints;
        self.watchpoints
            .remove_by_dqe(self.debugee.tracee_ctl(), breakpoints, dqe)
    }

    /// Return a list of all watchpoints.
    pub fn watchpoint_list(&self) -> Vec<WatchpointView<'_>> {
        self.watchpoints.all().iter().map(|wp| wp.into()).collect()
    }

    pub(super) fn execute_on_watchpoint_hook(
        &mut self,
        tid: Pid,
        pc: RelocatedAddress,
        ty: &WatchpointHitType,
    ) -> Result<(), Error> {
        match ty {
            WatchpointHitType::DebugRegister(reg) => {
                let maybe_wp = self
                    .watchpoints
                    .all()
                    .iter()
                    .find(|wp| wp.register() == Some(*reg) && !wp.temporary);

                if let Some(wp) = maybe_wp {
                    let number = wp.number();

                    match &wp.subject {
                        Subject::Expression(target) => {
                            let dqe = target.underlying_dqe().clone();
                            let current_tid = self.ecx().pid_on_focus();

                            let new_value = match target.frame_id {
                                None => {
                                    Watchpoint::execute_dqe(self, dqe).map(|qr| qr.into_value())
                                }
                                // frame_id is actual if current tid and expression tid are equals,
                                // otherwise evaluate as is
                                Some(_) if target.tid != current_tid => {
                                    Watchpoint::execute_dqe(self, dqe).map(|qr| qr.into_value())
                                }
                                Some(frame_id) => {
                                    let bt = self.backtrace(current_tid)?;
                                    let (num, frame) = bt
                                        .iter()
                                        .enumerate()
                                        .find(|(_, frame)| frame.id() == Some(frame_id))
                                        .ok_or(Error::VarFrameNotFound)?;

                                    let loc = Location::new(
                                        frame.ip,
                                        frame.ip.into_global(&self.debugee).unwrap(),
                                        current_tid,
                                    );
                                    let ecx = ExplorationContext::new(loc, num as u32);
                                    call_with_context(self, ecx, |debugger| {
                                        Watchpoint::execute_dqe(debugger, dqe)
                                            .map(|qr| qr.into_value())
                                    })
                                }
                            };

                            let new_value = new_value.ok();

                            let wp_mut = self
                                .watchpoints
                                .all_mut()
                                .iter_mut()
                                .find(|wp| wp.register() == Some(*reg))
                                .expect("infallible");
                            let Subject::Expression(t) = &mut wp_mut.subject else {
                                unreachable!()
                            };
                            let old = mem::replace(&mut t.last_value, new_value);

                            let dwarf = self.debugee.debug_info(pc)?;
                            let place = weak_error!(
                                dwarf.find_place_from_pc(pc.into_global(&self.debugee)?)
                            )
                            .flatten();

                            self.hooks
                                .on_watchpoint(
                                    pc,
                                    number,
                                    place,
                                    wp_mut.hw.condition,
                                    Some(&t.source_string),
                                    old.as_ref(),
                                    t.last_value.as_ref(),
                                    false,
                                )
                                .map_err(Hook)?;
                        }
                        Subject::Address(_) => {
                            let wp_mut = self
                                .watchpoints
                                .all_mut()
                                .iter_mut()
                                .find(|wp| wp.register() == Some(*reg))
                                .expect("infallible");
                            let Subject::Address(t) = &mut wp_mut.subject else {
                                unreachable!()
                            };
                            let old = t.refresh_last_value(tid, &wp_mut.hw);

                            let dwarf = self.debugee.debug_info(pc)?;
                            let place = weak_error!(
                                dwarf.find_place_from_pc(pc.into_global(&self.debugee)?)
                            )
                            .flatten();
                            self.hooks
                                .on_watchpoint(
                                    pc,
                                    number,
                                    place,
                                    wp_mut.hw.condition,
                                    None,
                                    old.as_ref(),
                                    t.last_value.as_ref(),
                                    false,
                                )
                                .map_err(Hook)?;
                        }
                    }
                }
            }
            WatchpointHitType::EndOfScope(wps) => {
                let watchpoints = wps
                    .iter()
                    .filter_map(|&num| self.watchpoints.get(num))
                    .collect::<Vec<_>>();
                debug_assert_eq!(watchpoints.len(), wps.len());

                let dwarf = self.debugee.debug_info(pc)?;
                let place =
                    weak_error!(dwarf.find_place_from_pc(pc.into_global(&self.debugee)?)).flatten();

                for wp in watchpoints {
                    let dqe_string =
                        if let Subject::Expression(ExpressionTarget { source_string, .. }) =
                            &wp.subject
                        {
                            Some(source_string.as_str())
                        } else {
                            None
                        };

                    self.hooks
                        .on_watchpoint(
                            pc,
                            wp.number(),
                            place.clone(),
                            wp.hw.condition,
                            dqe_string,
                            wp.last_value(),
                            None,
                            true,
                        )
                        .map_err(Hook)?;
                }
                for number in wps {
                    self.remove_watchpoint_by_number(*number)?;
                }
            }
        }

        Ok(())
    }
}
