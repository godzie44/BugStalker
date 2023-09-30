use crate::debugger::address::RelocatedAddress;
use crate::debugger::breakpoint::Breakpoint;
use crate::debugger::code;
use crate::debugger::debugee::tracee::{StopType, TraceeCtl, TraceeStatus};
use crate::debugger::error::Error;
use crate::debugger::error::Error::{MultipleErrors, ProcessExit, Ptrace, Waitpid};
use log::{debug, warn};
use nix::errno::Errno;
use nix::libc::pid_t;
use nix::sys::signal::{Signal, SIGSTOP};
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::Pid;
use nix::{libc, sys};
use std::collections::VecDeque;

/// List of signals that dont interrupt debugging process and send
/// to debugee directly on fire.
static QUIET_SIGNALS: [Signal; 6] = [
    Signal::SIGALRM,
    Signal::SIGURG,
    Signal::SIGCHLD,
    Signal::SIGIO,
    Signal::SIGVTALRM,
    Signal::SIGPROF,
    //Signal::SIGWINCH,
];

#[derive(Debug)]
pub enum StopReason {
    /// Whole debugee process exited with code
    DebugeeExit(i32),
    /// Debugee just started
    DebugeeStart,
    /// Debugee stopped at breakpoint
    Breakpoint(Pid, RelocatedAddress),
    /// Debugee stopped with OS signal
    SignalStop(Pid, Signal),
    /// Debugee stopped with Errno::ESRCH
    NoSuchProcess(Pid),
}

#[derive(Clone, Copy)]
pub struct TraceContext<'a> {
    pub breakpoints: &'a Vec<&'a Breakpoint>,
}

impl<'a> TraceContext<'a> {
    pub fn new(breakpoints: &'a Vec<&'a Breakpoint>) -> Self {
        Self { breakpoints }
    }
}

/// Ptrace tracer.
pub struct Tracer {
    pub(super) tracee_ctl: TraceeCtl,

    signal_queue: VecDeque<(Pid, Signal)>,
    group_stop_guard: bool,
}

impl Tracer {
    pub fn new(proc_pid: Pid) -> Self {
        Self {
            tracee_ctl: TraceeCtl::new(proc_pid),
            signal_queue: VecDeque::new(),
            group_stop_guard: false,
        }
    }

    /// Continue debugee execution until stop happened.
    pub fn resume(&mut self, ctx: TraceContext) -> Result<StopReason, Error> {
        loop {
            if let Some(req) = self.signal_queue.pop_front() {
                self.tracee_ctl.cont_stopped_ex(
                    Some(req),
                    self.signal_queue.iter().map(|(pid, _)| *pid).collect(),
                )?;

                if let Some((pid, sign)) = self.signal_queue.front().copied() {
                    // if there is more signal stop debugee again
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
                    return Ok(StopReason::NoSuchProcess(self.tracee_ctl.proc_pid()))
                }
                Err(e) => return Err(Waitpid(e)),
            };

            debug!(target: "tracer", "received new thread status: {status:?}");
            if let Some(stop) = self.apply_new_status(ctx, status)? {
                // if stop fired by quiet signal - go to next iteration, this will inject signal at
                // tracee process and resume it
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

        let non_stopped_exists = self
            .tracee_ctl
            .snapshot()
            .into_iter()
            .any(|t| t.pid != initiator_pid);
        if !non_stopped_exists {
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
                        Some(StopReason::Breakpoint(pid, _)) => {
                            // tracee already stopped cause breakpoint reached
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

                    // reload tracee, it state must be change after handle signal
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
                        // cause currently `fork()` in debugee is unsupported we expect this code calling once
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

                            let _new_thread_id = new_thread_id;
                            debug_assert!(
                                matches!(
                                new_trace_status,
                                WaitStatus::PtraceEvent(_new_thread_id, _, libc::PTRACE_EVENT_STOP)
                            ),
                                "the newly cloned thread must start with PTRACE_EVENT_STOP (cause PTRACE_SEIZE was used)"
                            )
                        }
                    }
                    libc::PTRACE_EVENT_STOP => {
                        // fire right after new thread started or PTRACE_INTERRUPT called.
                        match self.tracee_ctl.tracee_mut(pid) {
                            Some(tracee) => tracee.set_stop(StopType::Interrupt),
                            None => {
                                self.tracee_ctl.add(pid);
                            }
                        }
                    }
                    libc::PTRACE_EVENT_EXIT => {
                        // Stop the tracee at exit
                        let tracee = self.tracee_ctl.remove(pid);
                        if let Some(mut tracee) = tracee {
                            tracee.r#continue(None)?;
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

                            let has_tmp_breakpoints =
                                ctx.breakpoints.iter().any(|b| b.is_temporary());
                            if has_tmp_breakpoints {
                                let brkpt = ctx
                                    .breakpoints
                                    .iter()
                                    .find(|brkpt| brkpt.addr == current_pc)
                                    .unwrap();

                                if brkpt.is_temporary() && pid == brkpt.pid {
                                } else {
                                    let mut unusual_brkpt = (*brkpt).clone();
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

                            Ok(Some(StopReason::Breakpoint(pid, current_pc)))
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
                        self.signal_queue.push_back((pid, signal));
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
    /// returns: a [`None`] if instruction step done successfully. A [`StopReason::SignalStop`] returned
    /// if step interrupt cause tracee in a signal-stop. Error returned otherwise.
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

            // check that debugee step into expected trap (breakpoints ignored and are also considered as a trap)
            let in_trap = matches!(status, WaitStatus::Stopped(_, Signal::SIGTRAP))
                && (info.si_code == code::TRAP_TRACE
                    || info.si_code == code::TRAP_BRKPT
                    || info.si_code == code::SI_KERNEL);
            if in_trap {
                // check that we are not on original pc value
                if tracee.pc()? == initial_pc {
                    tracee.step(None)?;
                    continue;
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
