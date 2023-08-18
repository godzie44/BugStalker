use crate::debugger::address::{Address, GlobalAddress};
use crate::debugger::breakpoint::Breakpoint;
use crate::debugger::debugee::dwarf::unit::PlaceDescriptorOwned;
use crate::debugger::debugee::tracer::TraceContext;
use crate::debugger::{Debugger, ExplorationContext};
use anyhow::anyhow;

impl Debugger {
    /// Do single step (until debugee reaches a different source line).
    ///
    /// **! change exploration context**
    pub(super) fn step_in(&mut self) -> anyhow::Result<&ExplorationContext> {
        // make instruction step but ignoring functions prolog
        // initial function must exists (do instruction steps until it's not)
        fn long_step(debugger: &mut Debugger) -> anyhow::Result<PlaceDescriptorOwned> {
            loop {
                // initial step
                let ctx = debugger.single_step_instruction()?;

                let mut location = ctx.location();
                let func = loop {
                    let dwarf = debugger.debugee.debug_info(location.pc)?;
                    if let Some(func) = dwarf.find_function_by_pc(location.global_pc) {
                        break func;
                    }
                    let ctx = debugger.single_step_instruction()?;
                    location = ctx.location();
                };

                let prolog = func.prolog()?;
                // if pc in prolog range - step until function body is reached
                while debugger
                    .exploration_ctx()
                    .location()
                    .global_pc
                    .in_range(&prolog)
                {
                    debugger.single_step_instruction()?;
                }

                let location = debugger.exploration_ctx().location();
                if let Some(place) = debugger
                    .debugee
                    .debug_info(location.pc)?
                    .find_exact_place_from_pc(location.global_pc)
                {
                    return Ok(place.to_owned());
                }
            }
        }

        let mut location = self.exploration_ctx().location();

        let start_place = loop {
            let dwarf = &self.debugee.debug_info(location.pc)?;
            if let Some(place) = dwarf.find_place_from_pc(location.global_pc) {
                break place;
            }
            let ctx = self.single_step_instruction()?;
            location = ctx.location();
        };

        let sp_file = start_place.file.to_path_buf();
        let sp_line = start_place.line_number;
        let start_cfa = self
            .debugee
            .debug_info(location.pc)?
            .get_cfa(&self.debugee, &ExplorationContext::new(location, 0))?;

        loop {
            let next_place = long_step(self)?;
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

        self.expl_ctx_update_location()
    }

    /// Move debugee to next instruction, step over breakpoint if needed.
    ///
    /// **! change exploration context**
    pub(super) fn single_step_instruction(&mut self) -> anyhow::Result<&ExplorationContext> {
        let loc = self.exploration_ctx().location();
        if self.breakpoints.get_enabled(loc.pc).is_some() {
            Ok(self.step_over_breakpoint()?)
        } else {
            self.debugee.tracer_mut().single_step(
                TraceContext::new(&self.breakpoints.active_breakpoints()),
                loc.pid,
            )?;
            self.expl_ctx_update_location()
        }
    }

    /// If current on focus thread is stopped at a breakpoint, then it takes a step through this point.
    ///
    /// **! change exploration context**
    pub(super) fn step_over_breakpoint(&mut self) -> anyhow::Result<&ExplorationContext> {
        // cannot use debugee::Location mapping offset may be not init yet
        let tracee = self
            .debugee
            .get_tracee_ensure(self.exploration_ctx().pid_on_focus());
        let mb_brkpt = self.breakpoints.get_enabled(tracee.pc()?);
        let tracee_pid = tracee.pid;
        if let Some(brkpt) = mb_brkpt {
            if brkpt.is_enabled() {
                brkpt.disable()?;
                self.debugee.tracer_mut().single_step(
                    TraceContext::new(&self.breakpoints.active_breakpoints()),
                    tracee_pid,
                )?;
                brkpt.enable()?;
                return self.expl_ctx_update_location();
            }
        }
        Ok(self.exploration_ctx())
    }

    /// Move to higher stack frame.
    ///
    /// **! change exploration context**
    pub(super) fn step_out_frame(&mut self) -> anyhow::Result<()> {
        let ctx = self.exploration_ctx();
        let location = ctx.location();
        if let Some(ret_addr) = self.debugee.return_addr(ctx.pid_on_focus())? {
            let brkpt_is_set = self.breakpoints.get_enabled(ret_addr).is_some();
            if brkpt_is_set {
                self.continue_execution()?;
            } else {
                let brkpt = Breakpoint::new_temporary(ret_addr, location.pid);
                self.breakpoints.add_and_enable(brkpt)?;
                self.continue_execution()?;
                self.remove_breakpoint(Address::Relocated(ret_addr))?;
            }
        }
        self.expl_ctx_update_location()?;
        Ok(())
    }

    /// Do debugee step (over subroutine calls to).
    ///
    /// **! change exploration context**
    pub(super) fn step_over_any(&mut self) -> anyhow::Result<()> {
        let ctx = self.exploration_ctx();
        let mut current_location = ctx.location();

        let func = loop {
            let dwarf = &self.debugee.debug_info(current_location.pc)?;
            if let Some(func) = dwarf.find_function_by_pc(current_location.global_pc) {
                break func;
            }
            let ctx = self.single_step_instruction()?;
            current_location = ctx.location();
        };

        let prolog = func.prolog()?;
        let dwarf = &self.debugee.debug_info(current_location.pc)?;
        let inline_ranges = func.inline_ranges();

        let current_place = dwarf
            .find_place_from_pc(current_location.global_pc)
            .ok_or_else(|| anyhow!("current line not found"))?;

        let mut step_over_breakpoints = vec![];
        let mut to_delete = vec![];

        for range in func.ranges() {
            let mut place = func
                .unit()
                .find_place_by_pc(GlobalAddress::from(range.begin))
                .ok_or_else(|| anyhow!("unknown function range"))?;

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
                let in_inline_range = inline_ranges
                    .iter()
                    .any(|inline_range| place.address.in_range(inline_range));

                if !in_inline_range
                    && place.is_stmt
                    && place.address != current_place.address
                    && place.line_number != current_place.line_number
                {
                    let load_addr = place
                        .address
                        .relocate(self.debugee.mapping_offset_for_pc(current_location.pc)?);
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
                    .add_and_enable(Breakpoint::new_temporary(load_addr, current_location.pid))
                    .map(|_| ())
            })?;

        let return_addr = self.debugee.return_addr(current_location.pid)?;
        if let Some(ret_addr) = return_addr {
            if self.breakpoints.get_enabled(ret_addr).is_none() {
                self.breakpoints
                    .add_and_enable(Breakpoint::new_temporary(ret_addr, current_location.pid))?;
                to_delete.push(ret_addr);
            }
        }

        self.continue_execution()?;

        to_delete
            .into_iter()
            .try_for_each(|addr| self.remove_breakpoint(Address::Relocated(addr)).map(|_| ()))?;

        // if a step is taken outside and new location pc not equals to place pc
        // then we then we stopped at the place of the previous function call, and got into an assignment operation or similar
        // in this case do a single step
        let new_location = self.exploration_ctx().location();
        if Some(new_location.pc) == return_addr {
            let place = self
                .debugee
                .debug_info(new_location.pc)?
                .find_place_from_pc(new_location.global_pc)
                .ok_or_else(|| anyhow!("unknown function range"))?;

            if place.address != new_location.global_pc {
                self.step_in()?;
            }
        }

        self.expl_ctx_update_location()?;
        Ok(())
    }
}
