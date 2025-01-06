use crate::debugger::address::RelocatedAddress;
use crate::debugger::breakpoint::{Breakpoint, BrkptType};
use crate::debugger::debugee::tracee::{StopType, TraceeCtl, TraceeStatus};
use crate::debugger::error::Error;
use crate::debugger::error::Error::{MultipleErrors, ProcessExit, Ptrace, Waitpid};
use crate::debugger::register::debug::DebugRegisterNumber;
use crate::debugger::watchpoint::WatchpointRegistry;
use crate::debugger::{code, register};
use crate::weak_error;
use log::{debug, warn};
use nix::errno::Errno;
use nix::libc::pid_t;
use nix::sys::signal::{SIGSTOP, Signal};
use nix::sys::wait::{WaitStatus, waitpid};
use nix::unistd::Pid;
use nix::{libc, sys};
use std::collections::VecDeque;

/// List of signals that dont interrupt a debugging process and send
/// to debugee directly on fire.
static QUIET_SIGNALS: &[Signal] = &[
    Signal::SIGALRM,
    Signal::SIGURG,
    Signal::SIGCHLD,
    Signal::SIGIO,
    Signal::SIGVTALRM,
    Signal::SIGPROF,
    //Signal::SIGWINCH,
];

/// List of signals that may interrupt a debugging process but debugger will not inject it into.
static TRANSPARENT_SIGNALS: &[Signal] = &[Signal::SIGINT];

#[derive(Debug, Clone)]
pub enum WatchpointHitType {
    /// Hit of the underlying hardware breakpoint cause value changed.
    DebugRegister(DebugRegisterNumber),
    /// Hit of the underlying breakpoint at the end of the watchpoint scope.
    EndOfScope(Vec<u32>),
}

#[derive(Debug)]
pub enum StopReason {
    /// Whole debugee process exited with code.
    DebugeeExit(i32),
    /// Debugee just started.
    DebugeeStart,
    /// Debugee stopped at breakpoint.
    Breakpoint(Pid, RelocatedAddress),
    /// Debugee stopped at watchpoint.
    Watchpoint(Pid, RelocatedAddress, WatchpointHitType),
    /// Debugee stopped with OS signal.
    SignalStop(Pid, Signal),
    /// Debugee stopped with Errno::ESRCH.
    NoSuchProcess(Pid),
}

#[derive(Clone, Copy)]
pub struct TraceContext<'a> {
    pub breakpoints: &'a [&'a Breakpoint],
    pub watchpoints: &'a WatchpointRegistry,
}

impl<'a> TraceContext<'a> {
    pub fn new(
        breakpoints: &'a [&'a Breakpoint],
        watchpoint_registry: &'a WatchpointRegistry,
    ) -> Self {
        Self {
            breakpoints,
            watchpoints: watchpoint_registry,
        }
    }
}

/// Ptrace tracer.
pub struct Tracer {
    pub(super) tracee_ctl: TraceeCtl,

    inject_signal_queue: VecDeque<(Pid, Signal)>,
    group_stop_guard: bool,
}

impl Tracer {
    /// Create new [`Tracer`] for internally created debugee process.
    ///
    /// # Arguments
    ///
    /// * `proc_pid`: process id
    pub fn new(proc_pid: Pid) -> Self {
        Self {
            tracee_ctl: TraceeCtl::new(proc_pid),
            inject_signal_queue: VecDeque::new(),
            group_stop_guard: false,
        }
    }

    /// Create [`Tracer`] for external process attached by pid.
    ///
    /// # Arguments
    ///
    /// * `proc_pid`: process id
    /// * `threads`: id's of process threads
    pub fn new_external(proc_pid: Pid, threads: &[Pid]) -> Self {
        Self {
            tracee_ctl: TraceeCtl::new_external(proc_pid, threads),
            inject_signal_queue: VecDeque::new(),
            group_stop_guard: false,
        }
    }

    /// Continue debugee execution until stop happened.
    pub fn resume(&mut self, ctx: TraceContext) -> Result<StopReason, Error> {
        loop {
            if let Some(req) = self.inject_signal_queue.pop_front() {
                self.tracee_ctl.cont_stopped_ex(
                    Some(req),
                    self.inject_signal_queue
                        .iter()
                        .map(|(pid, _)| *pid)
                        .collect(),
                )?;

                if let Some((pid, sign)) = self.inject_signal_queue.front().copied() {
                    // if there are more signals - stop debugee again
                    self.group_stop_interrupt(ctx, Pid::from_raw(-1))?;
                    return Ok(StopReason::SignalStop(pid, sign));
                }
            } else {
                self.tracee_ctl.cont_stopped().map_err(MultipleErrors)?;
            }

            debug!(target: "tracer", "resume debugee execution, wait for updates");
            let status = match waitpid(Pid::from_raw(-1), None) {
                Ok(status) => status,
                Err(Errno::ECHILD) => {
                    return Ok(StopReason::NoSuchProcess(self.tracee_ctl.proc_pid()));
                }
                Err(e) => return Err(Waitpid(e)),
            };

            debug!(target: "tracer", "received new thread status: {status:?}");
            if let Some(stop) = self.apply_new_status(ctx, status)? {
                // if stop fired by quiet signal - go to next iteration, this will inject signal at
                // a tracee process and resume it
                if let StopReason::SignalStop(_, signal) = stop {
                    if QUIET_SIGNALS.contains(&signal) {
                        continue;
                    }
                }

                debug!(target: "tracer", "debugee stopped, reason: {stop:?}");
                return Ok(stop);
            }
        }
    }

    fn group_stop_in_progress(&self) -> bool {
        self.group_stop_guard
    }

    fn lock_group_stop(&mut self) {
        self.group_stop_guard = true
    }

    fn unlock_group_stop(&mut self) {
        self.group_stop_guard = false
    }

    /// For stop whole debugee process this function stops tracees (threads) one by one
    /// using PTRACE_INTERRUPT request.
    ///
    /// Stops only already running tracees.
    ///
    /// If tracee receives signals before interrupt - then tracee in signal-stop and no need to interrupt it.
    ///
    /// # Arguments
    ///
    /// * `initiator_pid`: tracee with this thread id already stopped, there is no need to interrupt it.
    fn group_stop_interrupt(&mut self, ctx: TraceContext, initiator_pid: Pid) -> Result<(), Error> {
        if self.group_stop_in_progress() {
            return Ok(());
        }
        self.lock_group_stop();

        debug!(
            target: "tracer",
            "initiate group stop, initiator: {initiator_pid}, debugee state: {:?}",
            self.tracee_ctl.snapshot()
        );

        let non_stopped_exist = self
            .tracee_ctl
            .tracee_iter()
            .any(|t| t.pid != initiator_pid);
        if !non_stopped_exist {
            // no need to group-stop
            debug!(
                target: "tracer",
                "group stop complete, debugee state: {:?}",
                self.tracee_ctl.snapshot()
            );
            self.unlock_group_stop();
            return Ok(());
        }

        // two rounds, cause may be new tracees at first round, they stopped at round 2
        for _ in 0..2 {
            let tracees = self.tracee_ctl.snapshot();

            for tid in tracees.into_iter().map(|t| t.pid) {
                // load current tracee snapshot
                let mut tracee = match self.tracee_ctl.tracee(tid) {
                    None => continue,
                    Some(tracee) => {
                        if tracee.is_stopped() {
                            continue;
                        } else {
                            tracee.clone()
                        }
                    }
                };

                if let Err(e) = sys::ptrace::interrupt(tracee.pid) {
                    // if no such process - continue, it will be removed later, on PTRACE_EVENT_EXIT event.
                    if Errno::ESRCH == e {
                        warn!("thread {} not found, ESRCH", tracee.pid);
                        if let Some(t) = self.tracee_ctl.tracee_mut(tracee.pid) {
                            t.set_stop(StopType::Interrupt);
                        }
                        continue;
                    }
                    return Err(Ptrace(e));
                }

                let mut wait = tracee.wait_one()?;

                while !matches!(wait, WaitStatus::PtraceEvent(_, _, libc::PTRACE_EVENT_STOP)) {
                    let stop = self.apply_new_status(ctx, wait)?;
                    match stop {
                        None => {}
                        Some(StopReason::Breakpoint(pid, _))
                        | Some(StopReason::Watchpoint(pid, _, _)) => {
                            // tracee already stopped cause breakpoint or watchpoint are reached
                            if pid == tracee.pid {
                                break;
                            }
                        }
                        Some(StopReason::DebugeeExit(code)) => return Err(ProcessExit(code)),
                        Some(StopReason::DebugeeStart) => {
                            unreachable!("stop at debugee entry point twice")
                        }
                        Some(StopReason::SignalStop(_, _)) => {
                            // tracee in signal-stop
                            break;
                        }
                        Some(StopReason::NoSuchProcess(_)) => {
                            // expect that tracee will be removed later
                            break;
                        }
                    }

                    // reload tracee, it states must be changed after handle signal
                    tracee = match self.tracee_ctl.tracee(tracee.pid).cloned() {
                        None => break,
                        Some(t) => t,
                    };
                    if tracee.is_stopped()
                        && matches!(tracee.status, TraceeStatus::Stopped(StopType::Interrupt))
                    {
                        break;
                    }

                    wait = tracee.wait_one()?;
                }

                if let Some(t) = self.tracee_ctl.tracee_mut(tracee.pid) {
                    if !t.is_stopped() {
                        t.set_stop(StopType::Interrupt);
                    }
                }
            }
        }

        self.unlock_group_stop();

        debug!(
            target: "tracer",
            "group stop complete, debugee state: {:?}",
            self.tracee_ctl.snapshot()
        );

        Ok(())
    }

    /// Handle tracee event fired by `wait` syscall.
    /// After this function ends tracee_ctl must be in consistent state.
    /// If debugee process stop detected - returns a stop reason.
    ///
    /// # Arguments
    ///
    /// * `status`: new status returned by `waitpid`.
    fn apply_new_status(
        &mut self,
        ctx: TraceContext,
        status: WaitStatus,
    ) -> Result<Option<StopReason>, Error> {
        match status {
            WaitStatus::Exited(pid, code) => {
                // Thread exited with tread id
                self.tracee_ctl.remove(pid);
                if pid == self.tracee_ctl.proc_pid() {
                    return Ok(Some(StopReason::DebugeeExit(code)));
                }
                Ok(None)
            }
            WaitStatus::PtraceEvent(pid, _signal, code) => {
                match code {
                    libc::PTRACE_EVENT_EXEC => {
                        // fire just before debugee start
                        // cause currently `fork()`
                        // in debugee is unsupported we expect this code to call once
                        self.tracee_ctl.add(pid);
                        return Ok(Some(StopReason::DebugeeStart));
                    }
                    libc::PTRACE_EVENT_CLONE => {
                        // fire just before new thread created
                        self.tracee_ctl
                            .tracee_ensure_mut(pid)
                            .set_stop(StopType::Interrupt);
                        let new_thread_id =
                            Pid::from_raw(sys::ptrace::getevent(pid).map_err(Ptrace)? as pid_t);

                        // PTRACE_EVENT_STOP may be received first, and new tracee may be already registered at this point
                        if self.tracee_ctl.tracee_mut(new_thread_id).is_none() {
                            let new_tracee = self.tracee_ctl.add(new_thread_id);
                            let new_trace_status = new_tracee.wait_one()?;
                            if matches!(new_trace_status, WaitStatus::Exited(_, _)) {
                                // this situation can occur if the process has already completed
                                self.tracee_ctl.remove(new_thread_id);
                            } else {
                                // all watchpoints must be distributed to a new tracee
                                weak_error!(ctx.watchpoints.distribute_to_tracee(new_tracee));

                                debug_assert!(
                                    matches!(
                                        new_trace_status,
                                        WaitStatus::PtraceEvent(tid, _, libc::PTRACE_EVENT_STOP) if tid == new_thread_id
                                    ),
                                    "the newly cloned thread must start with PTRACE_EVENT_STOP (cause PTRACE_SEIZE was used), got {new_trace_status:?}"
                                )
                            }
                        }
                    }
                    libc::PTRACE_EVENT_STOP => {
                        // fire right after new thread started or PTRACE_INTERRUPT called.
                        match self.tracee_ctl.tracee_mut(pid) {
                            Some(tracee) => tracee.set_stop(StopType::Interrupt),
                            None => {
                                let tracee = self.tracee_ctl.add(pid);
                                weak_error!(ctx.watchpoints.distribute_to_tracee(tracee));
                            }
                        }
                    }
                    libc::PTRACE_EVENT_EXIT => {
                        // Stop the tracee at exit
                        let tracee = self.tracee_ctl.remove(pid);
                        if let Some(mut tracee) = tracee {
                            // TODO
                            // There is one interesting situation, when tracee may not exist
                            // at this point (according to ptrace documentation, it must exist).
                            // Tracee not exist when thread created inside `std::thread::scoped`.
                            // This can be verified by running watchpoints functional tests.
                            // It is a flaky behavior, but sometimes an error
                            // will be returned at this point.
                            // Currently error here muted, but this behaviour NFR.
                            _ = tracee.r#continue(None);
                        }
                    }
                    _ => {
                        warn!("unsupported (ignored) ptrace event, code: {code}");
                    }
                }
                Ok(None)
            }
            WaitStatus::Stopped(pid, signal) => {
                let info = match sys::ptrace::getsiginfo(pid) {
                    Ok(info) => info,
                    Err(Errno::ESRCH) => return Ok(Some(StopReason::NoSuchProcess(pid))),
                    Err(e) => return Err(Ptrace(e)),
                };

                match signal {
                    Signal::SIGTRAP => match info.si_code {
                        code::TRAP_TRACE => {
                            todo!()
                        }
                        code::TRAP_BRKPT | code::SI_KERNEL => {
                            let current_pc = {
                                let tracee = self.tracee_ctl.tracee_ensure(pid);
                                tracee.set_pc(tracee.pc()?.as_u64() - 1)?;
                                tracee.pc()?
                            };

                            let mb_hit_brkpt = ctx
                                .breakpoints
                                .iter()
                                .find(|brkpt| brkpt.addr == current_pc);
                            debug_assert!(
                                mb_hit_brkpt.is_some(),
                                "the interrupt caught but the breakpoint was not found"
                            );
                            let Some(&brkpt) = mb_hit_brkpt else {
                                return Ok(None);
                            };

                            let has_tmp_breakpoints = ctx
                                .breakpoints
                                .iter()
                                .any(|b| b.is_temporary() | b.is_temporary_async());
                            if has_tmp_breakpoints {
                                let temporary_hit = brkpt.is_temporary() && pid == brkpt.pid;
                                let temporary_async_hit = brkpt.is_temporary_async();
                                let watchpoint_hit = brkpt.is_wp_companion();
                                if !temporary_hit && !watchpoint_hit && !temporary_async_hit {
                                    let mut unusual_brkpt = brkpt.clone();
                                    unusual_brkpt.pid = pid;
                                    if unusual_brkpt.is_enabled() {
                                        unusual_brkpt.disable()?;
                                        while self.single_step(ctx, pid)?.is_some() {}
                                        unusual_brkpt.enable()?;
                                    }
                                    self.tracee_ctl
                                        .tracee_ensure_mut(pid)
                                        .set_stop(StopType::Interrupt);

                                    return Ok(None);
                                }
                            }

                            self.tracee_ctl
                                .tracee_ensure_mut(pid)
                                .set_stop(StopType::Interrupt);
                            self.group_stop_interrupt(ctx, pid)?;

                            if let BrkptType::WatchpointCompanion(wps) = brkpt.r#type() {
                                return Ok(Some(StopReason::Watchpoint(
                                    pid,
                                    current_pc,
                                    WatchpointHitType::EndOfScope(wps.clone()),
                                )));
                            }

                            Ok(Some(StopReason::Breakpoint(pid, current_pc)))
                        }
                        code::TRAP_HWBKPT => {
                            let current_pc = {
                                let tracee = self.tracee_ctl.tracee_ensure(pid);
                                tracee.pc()?
                            };

                            self.tracee_ctl
                                .tracee_ensure_mut(pid)
                                .set_stop(StopType::Interrupt);
                            self.group_stop_interrupt(ctx, pid)?;

                            let mut state = register::debug::HardwareDebugState::current(pid)?;
                            let reg = state.dr6.detect_and_flush().expect("should exists");
                            state.sync(pid)?;
                            let hit_type = WatchpointHitType::DebugRegister(reg);
                            Ok(Some(StopReason::Watchpoint(pid, current_pc, hit_type)))
                        }
                        code => {
                            debug!(
                                target: "tracer",
                                "unexpected SIGTRAP code {code}",
                            );
                            Ok(None)
                        }
                    },
                    _ => {
                        if !TRANSPARENT_SIGNALS.contains(&signal) {
                            self.inject_signal_queue.push_back((pid, signal));
                        }

                        self.tracee_ctl
                            .tracee_ensure_mut(pid)
                            .set_stop(StopType::SignalStop(signal));

                        if !QUIET_SIGNALS.contains(&signal) {
                            self.group_stop_interrupt(ctx, pid)?;
                        }

                        Ok(Some(StopReason::SignalStop(pid, signal)))
                    }
                }
            }
            WaitStatus::Signaled(_, _, _) => Ok(None),
            _ => {
                warn!("unexpected wait status: {status:?}");
                Ok(None)
            }
        }
    }

    /// Execute next instruction, then stop with `TRAP_TRACE`.
    ///
    /// # Arguments
    ///
    /// * `ctx`: trace context
    /// * `pid`: tracee pid
    ///
    /// returns: [`None`] if an instruction step is done successfully.
    /// A [`StopReason::SignalStop`] returned if step interrupt causes tracee in a signal-stop.
    /// A [`StopReason::Watchpoint`] returned if step interrupt causes hardware breakpoint is hit.
    /// Error returned otherwise.
    pub fn single_step(
        &mut self,
        ctx: TraceContext,
        pid: Pid,
    ) -> Result<Option<StopReason>, Error> {
        let tracee = self.tracee_ctl.tracee_ensure(pid);
        let initial_pc = tracee.pc()?;
        tracee.step(None)?;

        let reason = loop {
            let tracee = self.tracee_ctl.tracee_ensure_mut(pid);
            let status = tracee.wait_one()?;
            let info = sys::ptrace::getsiginfo(pid).map_err(Ptrace)?;

            // check that debugee step into an expected trap
            // (breakpoints ignored and are also considered as a trap)
            let in_trap = matches!(status, WaitStatus::Stopped(_, Signal::SIGTRAP))
                && (info.si_code == code::TRAP_TRACE
                    || info.si_code == code::TRAP_BRKPT
                    || info.si_code == code::SI_KERNEL
                    || info.si_code == code::TRAP_HWBKPT);
            if in_trap {
                let pc = tracee.pc()?;
                // check that we aren't on original pc value
                if pc == initial_pc {
                    tracee.step(None)?;
                    continue;
                }

                let mut state = register::debug::HardwareDebugState::current(pid)?;
                let maybe_dr = state.dr6.detect_and_flush();
                state.sync(pid)?;
                if let Some(dr) = maybe_dr {
                    let hit_type = WatchpointHitType::DebugRegister(dr);
                    break Some(StopReason::Watchpoint(pid, pc, hit_type));
                }

                let mb_brkpt = ctx.breakpoints.iter().find(|brkpt| brkpt.addr == pc);
                if let Some(BrkptType::WatchpointCompanion(wps)) = mb_brkpt.map(|b| b.r#type()) {
                    let hit_type = WatchpointHitType::EndOfScope(wps.clone());
                    break Some(StopReason::Watchpoint(pid, pc, hit_type));
                }

                break None;
            }

            let in_trap =
                matches!(status, WaitStatus::Stopped(_, Signal::SIGTRAP)) && (info.si_code == 5);
            if in_trap {
                // if in syscall step to syscall end
                sys::ptrace::syscall(tracee.pid, None).map_err(Ptrace)?;
                let syscall_status = tracee.wait_one()?;
                debug_assert!(matches!(
                    syscall_status,
                    WaitStatus::Stopped(_, Signal::SIGTRAP)
                ));

                // then do step again
                tracee.step(None)?;

                continue;
            }

            let is_interrupt = matches!(
                status,
                WaitStatus::PtraceEvent(p, SIGSTOP, libc::PTRACE_EVENT_STOP) if pid == p,
            );
            if is_interrupt {
                break None;
            }

            let stop = self.apply_new_status(ctx, status)?;
            match stop {
                None => {}
                Some(StopReason::Breakpoint(_, _)) => {
                    unreachable!("breakpoints must be ignore");
                }
                Some(StopReason::Watchpoint(_, _, _)) => {
                    unreachable!("watchpoints must be ignore");
                }
                Some(StopReason::DebugeeExit(code)) => return Err(ProcessExit(code)),
                Some(StopReason::DebugeeStart) => {
                    unreachable!("stop at debugee entry point twice")
                }
                Some(StopReason::SignalStop(_, signal)) => {
                    if QUIET_SIGNALS.contains(&signal) {
                        self.tracee_ctl.tracee_ensure(pid).step(Some(signal))?;
                        continue;
                    }

                    // tracee in signal-stop
                    break stop;
                }
                Some(StopReason::NoSuchProcess(_)) => {
                    // expect that tracee will be removed later
                    break None;
                }
            }
        };
        Ok(reason)
    }
}
