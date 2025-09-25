use crate::debugger::address::{Address, GlobalAddress, RelocatedAddress};
use crate::debugger::breakpoint::Breakpoint;
use crate::debugger::debugee::dwarf::unit::PlaceDescriptorOwned;
use crate::debugger::debugee::tracer::{StopReason, TraceContext, WatchpointHitType};
use crate::debugger::error::Error;
use crate::debugger::error::Error::{NoFunctionRanges, PlaceNotFound, ProcessExit};
use crate::debugger::{Debugger, ExplorationContext};
use nix::sys::signal::Signal;
use nix::unistd::Pid;

/// Result of a step, if [`SignalInterrupt`] or [`WatchpointInterrupt`] then
/// a step process interrupted and the user should know about it.
/// If `quiet` set to `true` then no hooks should occur.
pub(super) enum StepResult {
    Done,
    SignalInterrupt {
        signal: Signal,
        quiet: bool,
    },
    WatchpointInterrupt {
        pid: Pid,
        addr: RelocatedAddress,
        ty: WatchpointHitType,
        quiet: bool,
    },
}

impl StepResult {
    fn signal_interrupt_quiet(signal: Signal) -> Self {
        Self::SignalInterrupt {
            signal,
            quiet: true,
        }
    }

    fn signal_interrupt(signal: Signal) -> Self {
        Self::SignalInterrupt {
            signal,
            quiet: false,
        }
    }

    fn wp_interrupt_quite(pid: Pid, addr: RelocatedAddress, ty: WatchpointHitType) -> Self {
        Self::WatchpointInterrupt {
            pid,
            addr,
            ty,
            quiet: true,
        }
    }

    fn wp_interrupt(pid: Pid, addr: RelocatedAddress, ty: WatchpointHitType) -> Self {
        Self::WatchpointInterrupt {
            pid,
            addr,
            ty,
            quiet: false,
        }
    }
}

impl Debugger {
    /// Do a single step (until debugee reaches a different source line).
    ///
    /// Returns [`StepResult::SignalInterrupt`] if the step is interrupted by a signal
    /// or [`StepResult::Done`] if a step is done.
    ///
    /// **! change exploration context**
    pub(super) fn step_in(&mut self) -> Result<StepResult, Error> {
        enum PlaceOrStop {
            Place(PlaceDescriptorOwned),
            Signal(Signal),
            Watchpoint(Pid, RelocatedAddress, WatchpointHitType),
        }

        // make an instruction step but ignoring functions prolog
        // initial function must exist (do instruction steps until it's not)
        // returns stop place or signal if a step is undone
        fn step_over_prolog(debugger: &mut Debugger) -> Result<PlaceOrStop, Error> {
            macro_rules! prolog_single_step {
                ($debugger: expr) => {
                    match $debugger.single_step_instruction()? {
                        Some(StopReason::SignalStop(_, sign)) => {
                            return Ok(PlaceOrStop::Signal(sign));
                        }
                        Some(StopReason::Watchpoint(pid, addr, ty)) => {
                            return Ok(PlaceOrStop::Watchpoint(pid, addr, ty));
                        }
                        _ => {}
                    }
                };
            }

            loop {
                // initial step
                prolog_single_step!(debugger);
                let ecx = debugger.ecx();
                let mut location = ecx.location();
                // determine current function, if no debug information for function - step until function found
                let func = loop {
                    let dwarf = debugger.debugee.debug_info(location.pc)?;
                    // step's stop only if there is debug information for PC and current function can be determined
                    if let Ok(Some((func, _))) = dwarf.find_function_by_pc(location.global_pc) {
                        break func;
                    }
                    prolog_single_step!(debugger);
                    let ecx = debugger.ecx();
                    location = ecx.location();
                };

                let prolog = func.prolog()?;
                // if PC in prolog range - step until function body is reached
                while debugger.ecx().location().global_pc.in_range(&prolog) {
                    prolog_single_step!(debugger);
                }

                let location = debugger.ecx().location();
                if let Some(place) = debugger
                    .debugee
                    .debug_info(location.pc)?
                    .find_exact_place_from_pc(location.global_pc)?
                {
                    return Ok(PlaceOrStop::Place(place.to_owned()));
                }
            }
        }

        let mut location = self.ecx().location();

        let start_place = loop {
            let dwarf = &self.debugee.debug_info(location.pc)?;
            if let Ok(Some(place)) = dwarf.find_place_from_pc(location.global_pc) {
                break place;
            }
            match self.single_step_instruction()? {
                Some(StopReason::SignalStop(_, sign)) => {
                    return Ok(StepResult::signal_interrupt(sign));
                }
                Some(StopReason::Watchpoint(pid, addr, ty)) => {
                    return Ok(StepResult::wp_interrupt(pid, addr, ty));
                }
                _ => {}
            }
            location = self.ecx().location();
        };

        let sp_file = start_place.file.to_path_buf();
        let sp_line = start_place.line_number;
        let start_cfa = self
            .debugee
            .debug_info(location.pc)?
            .get_cfa(&self.debugee, &ExplorationContext::new(location, 0))?;

        loop {
            let next_place = match step_over_prolog(self)? {
                PlaceOrStop::Place(place) => place,
                PlaceOrStop::Signal(signal) => return Ok(StepResult::signal_interrupt(signal)),
                PlaceOrStop::Watchpoint(pid, addr, dr) => {
                    return Ok(StepResult::wp_interrupt(pid, addr, dr));
                }
            };
            if !next_place.is_stmt {
                continue;
            }
            let in_same_place = sp_file == next_place.file && sp_line == next_place.line_number;
            let location = self.ecx().location();
            let next_cfa = self
                .debugee
                .debug_info(location.pc)?
                .get_cfa(&self.debugee, &ExplorationContext::new(location, 0))?;

            // step is done if:
            // 1) we may step at same place in code but in another stack frame
            // 2) we step at another place in code (file + line)
            if start_cfa != next_cfa || !in_same_place {
                break;
            }
        }

        self.ecx_update_location()?;
        Ok(StepResult::Done)
    }

    /// Move debugee to next instruction, step over breakpoint if needed.
    /// May return a [`StopReason::SignalStop`] if the step didn't happen cause signal.
    ///
    /// **! change exploration context**
    pub(super) fn single_step_instruction(&mut self) -> Result<Option<StopReason>, Error> {
        let loc = self.ecx().location();
        let mb_reason = if self.breakpoints.get_enabled(loc.pc).is_some() {
            self.step_over_breakpoint()?
        } else {
            let maybe_reason = self.debugee.tracer_mut().single_step(
                TraceContext::new(&self.breakpoints.active_breakpoints(), &self.watchpoints),
                loc.pid,
            )?;
            self.ecx_update_location()?;
            maybe_reason
        };
        Ok(mb_reason)
    }

    /// If current on focus thread is stopped at a breakpoint, then it takes a step through this point.
    ///
    /// May return a [`StopReason::SignalStop`] or [`StopReason::Watchpoint`]
    /// if the step didn't happen cause signal or watchpoint is hit.
    ///
    /// **! change exploration context**
    pub(super) fn step_over_breakpoint(&mut self) -> Result<Option<StopReason>, Error> {
        // cannot use debugee::Location mapping offset may be not init yet
        let tracee = self.debugee.get_tracee_ensure(self.ecx().pid_on_focus());
        let mb_brkpt = self.breakpoints.get_enabled(tracee.pc()?);
        let tracee_pid = tracee.pid;
        if let Some(brkpt) = mb_brkpt
            && brkpt.is_enabled()
        {
            brkpt.disable()?;
            let maybe_reason = self.debugee.tracer_mut().single_step(
                TraceContext::new(&self.breakpoints.active_breakpoints(), &self.watchpoints),
                tracee_pid,
            )?;
            brkpt.enable()?;
            self.ecx_update_location()?;
            return Ok(maybe_reason);
        }
        Ok(None)
    }

    /// Move to higher stack frame.
    ///
    /// **! change exploration context**
    pub(super) fn step_out_frame(&mut self) -> Result<(), Error> {
        let ecx = self.ecx();
        let location = ecx.location();
        let debug_info = self.debugee.debug_info(location.pc)?;

        if let Some(ret_addr) = self.debugee.return_addr(ecx.pid_on_focus())? {
            let brkpt_is_set = self.breakpoints.get_enabled(ret_addr).is_some();
            if brkpt_is_set {
                self.continue_execution()?;
            } else {
                let brkpt =
                    Breakpoint::new_temporary(debug_info.pathname(), ret_addr, location.pid);
                self.breakpoints.add_and_enable(brkpt)?;
                self.continue_execution()?;
                self.remove_breakpoint(Address::Relocated(ret_addr))?;
            }
        }

        if self.debugee.is_exited() {
            // todo add exit code here
            return Err(ProcessExit(0));
        }

        self.ecx_update_location()?;
        Ok(())
    }

    /// Do debugee step (over subroutine calls too).
    /// Returns [`StepResult::SignalInterrupt`] if the step is interrupted by a signal
    /// or [`StepResult::Done`] if step done.
    ///
    /// **! change exploration context**
    pub(super) fn step_over_any(&mut self) -> Result<StepResult, Error> {
        let ecx = self.ecx();
        let mut current_location = ecx.location();

        // determine current function, if no debug information for function - step until function found
        let (func, info) = loop {
            let dwarf = &self.debugee.debug_info(current_location.pc)?;
            // step's stop only if there is debug information for PC and current function can be determined
            if let Ok(Some((func, info))) = dwarf.find_function_by_pc(current_location.global_pc) {
                break (func, info);
            }
            match self.single_step_instruction()? {
                Some(StopReason::SignalStop(_, sign)) => {
                    return Ok(StepResult::signal_interrupt(sign));
                }
                Some(StopReason::Watchpoint(pid, addr, ty)) => {
                    return Ok(StepResult::wp_interrupt(pid, addr, ty));
                }
                _ => {}
            }
            current_location = self.ecx().location();
        };
        let fn_file = info.decl_file_line.map(|fl| fl.0);

        let prolog = func.prolog()?;
        let dwarf = &self.debugee.debug_info(current_location.pc)?;
        let inline_ranges = func.inline_ranges();

        let current_place = dwarf
            .find_place_from_pc(current_location.global_pc)?
            .ok_or(PlaceNotFound(current_location.global_pc))?;

        let mut step_over_breakpoints = vec![];
        let mut to_delete = vec![];

        let fn_full_name = info.full_name();
        for range in func.ranges() {
            let mut place = func
                .unit()
                .find_place_by_pc(GlobalAddress::from(range.begin))
                .ok_or_else(|| NoFunctionRanges(fn_full_name.clone()))?;

            while place.address.in_range(&range) {
                if Some(place.file_idx) != fn_file {
                    match place.next() {
                        None => break,
                        Some(n) => place = n,
                    }
                    continue;
                }

                // skip places in function prolog
                if place.address.in_range(&prolog) {
                    match place.next() {
                        None => break,
                        Some(n) => place = n,
                    }
                    continue;
                }

                // guard against a step at inlined function body
                let in_inline_range = place.address.in_ranges(&inline_ranges);

                if !in_inline_range
                    && place.is_stmt
                    && place.address != current_place.address
                    && place.line_number != current_place.line_number
                {
                    let load_addr = place
                        .address
                        .relocate_to_segment_by_pc(&self.debugee, current_location.pc)?;
                    if self.breakpoints.get_enabled(load_addr).is_none() {
                        step_over_breakpoints.push(load_addr);
                        to_delete.push(load_addr);
                    }
                }

                match place.next() {
                    None => break,
                    Some(n) => place = n,
                }
            }
        }

        step_over_breakpoints
            .into_iter()
            .try_for_each(|load_addr| {
                self.breakpoints
                    .add_and_enable(Breakpoint::new_temporary(
                        dwarf.pathname(),
                        load_addr,
                        current_location.pid,
                    ))
                    .map(|_| ())
            })?;

        let return_addr = self.debugee.return_addr(current_location.pid)?;
        if let Some(ret_addr) = return_addr
            && self.breakpoints.get_enabled(ret_addr).is_none()
        {
            self.breakpoints.add_and_enable(Breakpoint::new_temporary(
                dwarf.pathname(),
                ret_addr,
                current_location.pid,
            ))?;
            to_delete.push(ret_addr);
        }

        let stop_reason = self.continue_execution()?;

        to_delete
            .into_iter()
            .try_for_each(|addr| self.remove_breakpoint(Address::Relocated(addr)).map(|_| ()))?;

        // hooks already called at [`Self::continue_execution`], so use `quite` opt
        match stop_reason {
            StopReason::SignalStop(_, sign) => {
                return Ok(StepResult::signal_interrupt_quiet(sign));
            }
            StopReason::Watchpoint(pid, addr, ty) => {
                return Ok(StepResult::wp_interrupt_quite(pid, addr, ty));
            }
            _ => {}
        }

        // if a step is taken outside and new location pc not equals to place pc,
        // then we stopped at the place of the previous function call,
        // and got into an assignment operation or similar in this case do a single step
        let new_location = self.ecx().location();
        if Some(new_location.pc) == return_addr {
            let place = self
                .debugee
                .debug_info(new_location.pc)?
                .find_place_from_pc(new_location.global_pc)?
                .ok_or_else(|| NoFunctionRanges(fn_full_name))?;
            if place.address != new_location.global_pc {
                match self.step_in()? {
                    StepResult::SignalInterrupt { signal, .. } => {
                        return Ok(StepResult::signal_interrupt(signal));
                    }
                    StepResult::WatchpointInterrupt { pid, addr, ty, .. } => {
                        return Ok(StepResult::wp_interrupt(pid, addr, ty));
                    }
                    _ => {}
                }
            }
        }

        if self.debugee.is_exited() {
            // todo add exit code here
            return Err(ProcessExit(0));
        }

        self.ecx_update_location()?;
        Ok(StepResult::Done)
    }
}
