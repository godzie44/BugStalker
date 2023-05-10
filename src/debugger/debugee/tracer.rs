use crate::debugger::address::RelocatedAddress;
use crate::debugger::code;
use crate::debugger::debugee::tracee::{StopType, TraceeCtl};
use anyhow::bail;
use ctrlc::Signal;
use log::{debug, warn};
use nix::errno::Errno;
use nix::libc::pid_t;
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::Pid;
use nix::{libc, sys};
use std::collections::VecDeque;

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
    pub fn resume(&mut self) -> anyhow::Result<StopReason> {
        loop {
            if let Some(req) = self.signal_queue.pop_front() {
                self.tracee_ctl.cont_stopped_ex(
                    Some(req),
                    self.signal_queue.iter().map(|(pid, _)| *pid).collect(),
                )?;

                if let Some((pid, sign)) = self.signal_queue.front().copied() {
                    // if there is more signal stop debugee again
                    self.group_stop_interrupt(Pid::from_raw(-1))?;
                    return Ok(StopReason::SignalStop(pid, sign));
                }
            } else {
                self.tracee_ctl.cont_stopped()?;
            }

            let status = waitpid(Pid::from_raw(-1), None)?;
            debug!("received new thread status: {status:?}");
            if let Some(stop) = self.update_state(status)? {
                debug!("debugee stopped, reason: {stop:?}");
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
    fn group_stop_interrupt(&mut self, initiator_pid: Pid) -> anyhow::Result<()> {
        if self.group_stop_in_progress() {
            return Ok(());
        }
        self.lock_group_stop();

        debug!(
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
                "group stop complete, debugee state: {:?}",
                self.tracee_ctl.snapshot()
            );
            self.unlock_group_stop();
            return Ok(());
        }

        for _ in 0..2 {
            let tracees = self.tracee_ctl.snapshot();

            for mut tracee in tracees {
                if tracee.is_stopped() {
                    continue;
                }

                if let Err(e) = sys::ptrace::interrupt(tracee.pid) {
                    // if no such process - continue, it will be removed later, on PTRACE_EVENT_EXIT event.
                    if Errno::ESRCH == e {
                        warn!("thread {} not found, ESRCH", tracee.pid);
                        if let Some(t) = self.tracee_ctl.tracee_mut(tracee.pid) {
                            t.set_stop(StopType::Interrupt);
                        }
                        continue;
                    }
                    bail!(anyhow::Error::from(e).context(format!("thread: {}", tracee.pid)));
                }

                let mut wait = tracee.wait_one()?;

                while !matches!(wait, WaitStatus::PtraceEvent(_, _, libc::PTRACE_EVENT_STOP)) {
                    let stop = { self.update_state(wait)? };
                    match stop {
                        None => {}
                        Some(StopReason::Breakpoint(_, _)) => {
                            todo!("handle this situation")
                        }
                        Some(StopReason::DebugeeExit(code)) => {
                            bail!("debugee process exit with {code}")
                        }
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
                    if tracee.is_stopped() {
                        tracee.r#continue(None)?;
                    }

                    // todo check still alive ?
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
            "group stop complete, debugee state: {:?}",
            self.tracee_ctl.snapshot()
        );

        Ok(())
    }

    /// Handle tracee event wired by `wait` syscall.
    /// After this function ends tracee_ctl must be in consistent state.
    /// If debugee process stop detected - returns stop reason.
    ///
    /// # Arguments
    ///
    /// * `status`: new status returned by `waitpid`.
    fn update_state(&mut self, status: WaitStatus) -> anyhow::Result<Option<StopReason>> {
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
                        let new_thread_id = Pid::from_raw(sys::ptrace::getevent(pid)? as pid_t);

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
                    Err(e) => return Err(e.into()),
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

                            self.tracee_ctl.set_tracee_to_focus(pid);
                            self.tracee_ctl
                                .tracee_ensure_mut(pid)
                                .set_stop(StopType::Interrupt);
                            self.group_stop_interrupt(pid)?;

                            Ok(Some(StopReason::Breakpoint(pid, current_pc)))
                        }
                        code => bail!("unexpected SIGTRAP code {code}"),
                    },
                    _ => {
                        self.signal_queue.push_back((pid, signal));
                        self.tracee_ctl
                            .tracee_ensure_mut(pid)
                            .set_stop(StopType::SignalStop(signal));
                        self.group_stop_interrupt(pid)?;

                        Ok(Some(StopReason::SignalStop(pid, signal)))
                    }
                }
            }
            _ => {
                warn!("unexpected wait status: {status:?}");
                Ok(None)
            }
        }
    }
}
