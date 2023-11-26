use crate::debugger::address::{Address, GlobalAddress};
use crate::debugger::breakpoint::Breakpoint;
use crate::debugger::debugee::dwarf::unit::PlaceDescriptorOwned;
use crate::debugger::debugee::tracer::{StopReason, TraceContext};
use crate::debugger::error::Error;
use crate::debugger::error::Error::{NoFunctionRanges, PlaceNotFound, ProcessExit};
use crate::debugger::{Debugger, ExplorationContext};
use nix::sys::signal::Signal;

/// Result of a step, if [`SignalInterrupt`] then step process interrupted by a signal and user must know it.
/// If `quiet` set to `true` than no hooks must occurred.
pub(super) enum StepResult {
    Done,
    SignalInterrupt { signal: Signal, quiet: bool },
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
}

impl Debugger {
    /// Do single step (until debugee reaches a different source line).
    /// Returns [`StepResult::SignalInterrupt`] if the step is interrupted by a signal
    /// or [`StepResult::Done`] if step done.
    ///
    /// **! change exploration context**
    pub(super) fn step_in(&mut self) -> Result<StepResult, Error> {
        enum PlaceOrSignal {
            Place(PlaceDescriptorOwned),
            Signal(Signal),
        }

        // make instruction step but ignoring functions prolog
        // initial function must exists (do instruction steps until it's not)
        // returns stop place or signal if step is undone
        fn step_over_prolog(debugger: &mut Debugger) -> Result<PlaceOrSignal, Error> {
            loop {
                // initial step
                if let Some(StopReason::SignalStop(_, sign)) = debugger.single_step_instruction()? {
                    return Ok(PlaceOrSignal::Signal(sign));
                }
                let ctx = debugger.exploration_ctx();
                let mut location = ctx.location();
                // determine current function, if no debug information for function - step until function found
                let func = loop {
                    let dwarf = debugger.debugee.debug_info(location.pc)?;
                    // step's stop only if there is a debug information for PC and current function can be determined
                    if let Ok(Some(func)) = dwarf.find_function_by_pc(location.global_pc) {
                        break func;
                    }
                    if let Some(StopReason::SignalStop(_, sign)) =
                        debugger.single_step_instruction()?
                    {
                        return Ok(PlaceOrSignal::Signal(sign));
                    }

                    let ctx = debugger.exploration_ctx();
                    location = ctx.location();
                };

                let prolog = func.prolog()?;
                // if PC in prolog range - step until function body is reached
                while debugger
                    .exploration_ctx()
                    .location()
                    .global_pc
                    .in_range(&prolog)
                {
                    if let Some(StopReason::SignalStop(_, sign)) =
                        debugger.single_step_instruction()?
                    {
                        return Ok(PlaceOrSignal::Signal(sign));
                    }
                }

                let location = debugger.exploration_ctx().location();
                if let Some(place) = debugger
                    .debugee
                    .debug_info(location.pc)?
                    .find_exact_place_from_pc(location.global_pc)?
                {
                    return Ok(PlaceOrSignal::Place(place.to_owned()));
                }
            }
        }

        let mut location = self.exploration_ctx().location();

        let start_place = loop {
            let dwarf = &self.debugee.debug_info(location.pc)?;
            if let Ok(Some(place)) = dwarf.find_place_from_pc(location.global_pc) {
                break place;
            }
            if let Some(StopReason::SignalStop(_, sign)) = self.single_step_instruction()? {
                return Ok(StepResult::signal_interrupt(sign));
            }
            location = self.exploration_ctx().location();
        };

        let sp_file = start_place.file.to_path_buf();
        let sp_line = start_place.line_number;
        let start_cfa = self
            .debugee
            .debug_info(location.pc)?
            .get_cfa(&self.debugee, &ExplorationContext::new(location, 0))?;

        loop {
            let next_place = match step_over_prolog(self)? {
                PlaceOrSignal::Place(place) => place,
                PlaceOrSignal::Signal(signal) => return Ok(StepResult::signal_interrupt(signal)),
            };
            if !next_place.is_stmt {
                continue;
            }
            let in_same_place = sp_file == next_place.file && sp_line == next_place.line_number;
            let location = self.exploration_ctx().location();
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

        self.expl_ctx_update_location()?;
        Ok(StepResult::Done)
    }

    /// Move debugee to next instruction, step over breakpoint if needed.
    /// May return a [`StopReason::SignalStop`] if the step didn't happen cause signal.
    ///
    /// **! change exploration context**
    pub(super) fn single_step_instruction(&mut self) -> Result<Option<StopReason>, Error> {
        let loc = self.exploration_ctx().location();
        let mb_signal = if self.breakpoints.get_enabled(loc.pc).is_some() {
            self.step_over_breakpoint()?
        } else {
            let mb_signal = self.debugee.tracer_mut().single_step(
                TraceContext::new(&self.breakpoints.active_breakpoints()),
                loc.pid,
            )?;
            self.expl_ctx_update_location()?;
            mb_signal
        };
        Ok(mb_signal)
    }

    /// If current on focus thread is stopped at a breakpoint, then it takes a step through this point.
    /// May return a [`StopReason::SignalStop`] if the step didn't happen cause signal.
    ///
    /// **! change exploration context**
    pub(super) fn step_over_breakpoint(&mut self) -> Result<Option<StopReason>, Error> {
        // cannot use debugee::Location mapping offset may be not init yet
        let tracee = self
            .debugee
            .get_tracee_ensure(self.exploration_ctx().pid_on_focus());
        let mb_brkpt = self.breakpoints.get_enabled(tracee.pc()?);
        let tracee_pid = tracee.pid;
        if let Some(brkpt) = mb_brkpt {
            if brkpt.is_enabled() {
                brkpt.disable()?;
                let mb_signal = self.debugee.tracer_mut().single_step(
                    TraceContext::new(&self.breakpoints.active_breakpoints()),
                    tracee_pid,
                )?;
                brkpt.enable()?;
                self.expl_ctx_update_location()?;
                return Ok(mb_signal);
            }
        }
        Ok(None)
    }

    /// Move to higher stack frame.
    ///
    /// **! change exploration context**
    pub(super) fn step_out_frame(&mut self) -> Result<(), Error> {
        let ctx = self.exploration_ctx();
        let location = ctx.location();
        let debug_info = self.debugee.debug_info(location.pc)?;

        if let Some(ret_addr) = self.debugee.return_addr(ctx.pid_on_focus())? {
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
        self.expl_ctx_update_location()?;
        Ok(())
    }

    /// Do debugee step (over subroutine calls too).
    /// Returns [`StepResult::SignalInterrupt`] if the step is interrupted by a signal
    /// or [`StepResult::Done`] if step done.
    ///
    /// **! change exploration context**
    pub(super) fn step_over_any(&mut self) -> Result<StepResult, Error> {
        let ctx = self.exploration_ctx();
        let mut current_location = ctx.location();

        // determine current function, if no debug information for function - step until function found
        let func = loop {
            let dwarf = &self.debugee.debug_info(current_location.pc)?;
            // step's stop only if there is a debug information for PC and current function can be determined
            if let Ok(Some(func)) = dwarf.find_function_by_pc(current_location.global_pc) {
                break func;
            }
            if let Some(StopReason::SignalStop(_, signal)) = self.single_step_instruction()? {
                return Ok(StepResult::signal_interrupt(signal));
            }
            current_location = self.exploration_ctx().location();
        };

        let prolog = func.prolog()?;
        let dwarf = &self.debugee.debug_info(current_location.pc)?;
        let inline_ranges = func.inline_ranges();

        let current_place = dwarf
            .find_place_from_pc(current_location.global_pc)?
            .ok_or(PlaceNotFound(current_location.global_pc))?;

        let mut step_over_breakpoints = vec![];
        let mut to_delete = vec![];

        let fn_full_name = func.full_name();
        for range in func.ranges() {
            let mut place = func
                .unit()
                .find_place_by_pc(GlobalAddress::from(range.begin))
                .ok_or_else(|| NoFunctionRanges(fn_full_name.clone()))?;

            while place.address.in_range(range) {
                // skip places in function prolog
                if place.address.in_range(&prolog) {
                    match place.next() {
                        None => break,
                        Some(n) => place = n,
                    }
                    continue;
                }

                // guard from step at inlined function body
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
        if let Some(ret_addr) = return_addr {
            if self.breakpoints.get_enabled(ret_addr).is_none() {
                self.breakpoints.add_and_enable(Breakpoint::new_temporary(
                    dwarf.pathname(),
                    ret_addr,
                    current_location.pid,
                ))?;
                to_delete.push(ret_addr);
            }
        }

        let stop_reason = self.continue_execution()?;

        to_delete
            .into_iter()
            .try_for_each(|addr| self.remove_breakpoint(Address::Relocated(addr)).map(|_| ()))?;

        if let StopReason::SignalStop(_, signal) = stop_reason {
            // on signal hook already called at [`Self::continue_execution`]
            return Ok(StepResult::signal_interrupt_quiet(signal));
        }

        // if a step is taken outside and new location pc not equals to place pc
        // then we then we stopped at the place of the previous function call, and got into an assignment operation or similar
        // in this case do a single step
        let new_location = self.exploration_ctx().location();
        if Some(new_location.pc) == return_addr {
            let place = self
                .debugee
                .debug_info(new_location.pc)?
                .find_place_from_pc(new_location.global_pc)?
                .ok_or_else(|| NoFunctionRanges(fn_full_name))?;
            if place.address != new_location.global_pc {
                if let StepResult::SignalInterrupt { signal, .. } = self.step_in()? {
                    return Ok(StepResult::signal_interrupt(signal));
                }
            }
        }

        if self.debugee.is_exited() {
            // todo add exit code here
            return Err(ProcessExit(0));
        }

        self.expl_ctx_update_location()?;
        Ok(StepResult::Done)
    }
}
