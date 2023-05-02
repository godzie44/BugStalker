use crate::debugger::address::RelocatedAddress;
use crate::debugger::code;
use crate::debugger::debugee::tracee::TraceeCtl;
use crate::debugger::debugee::tracee::TraceeStatus::{Running, Stopped};
use anyhow::bail;
use ctrlc::Signal;
use log::warn;
use nix::errno::Errno;
use nix::libc::pid_t;
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::Pid;
use nix::{libc, sys};

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
}

impl Tracer {
    pub fn new(proc_pid: Pid) -> Self {
        Self {
            tracee_ctl: TraceeCtl::new(proc_pid),
        }
    }

    /// Continue debugee execution until stop happened.
    pub fn continue_until_stop(&mut self) -> anyhow::Result<StopReason> {
        loop {
            self.tracee_ctl.cont_stopped()?;
            let status = waitpid(Pid::from_raw(-1), None)?;
            if let Some(stop) = self.update_state(status)? {
                return Ok(stop);
            }
        }
    }

    /// For stop whole debugee process this function stops tracees (threads) one by one
    /// using PTRACE_INTERRUPT request.
    /// If tracee receives signals before interrupt - handle signals.
    /// If signal initiate - stop - todo.
    fn group_stop_interrupt(&mut self, initiator_pid: Pid) -> anyhow::Result<()> {
        self.tracee_ctl.tracee_ensure_mut(initiator_pid).stop();

        let has_non_stopped = self
            .tracee_ctl
            .snapshot()
            .into_iter()
            .any(|t| t.pid != initiator_pid);
        if !has_non_stopped {
            // no need to group-stop
            return Ok(());
        }

        for _ in 0..2 {
            let tracees = self.tracee_ctl.snapshot();

            for tracee in tracees {
                if tracee.status == Running {
                    if let Err(e) = sys::ptrace::interrupt(tracee.pid) {
                        // if no such process - continue, it will be removed later, on PTRACE_EVENT_EXIT event.
                        if Errno::ESRCH == e {
                            warn!("thread {} not found, ESRCH", tracee.pid);
                            if let Some(t) = self.tracee_ctl.tracee_mut(tracee.pid) {
                                t.stop();
                            }
                            continue;
                        }
                        bail!(anyhow::Error::from(e).context(format!("thread: {}", tracee.pid)));
                    }

                    let mut tracee = tracee;
                    let mut wait = tracee.wait_one()?;

                    while !matches!(wait, WaitStatus::PtraceEvent(_, _, libc::PTRACE_EVENT_STOP)) {
                        let stop = { self.update_state(wait)? };
                        match stop {
                            None => {}
                            Some(StopReason::SignalStop(_, sig)) => {
                                // tracee in signal-stop, inject signal
                                tracee.r#continue(Some(sig))?;
                            }
                            Some(StopReason::Breakpoint(_, _)) => {
                                todo!("handle this situation")
                            }
                            Some(StopReason::DebugeeExit(code)) => {
                                bail!("debugee process exit with {code}")
                            }
                            Some(StopReason::DebugeeStart) => {
                                unreachable!("stop at debugee entry point twice")
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
                        if tracee.status == Stopped {
                            tracee.r#continue(None)?;
                        }

                        // todo check still alive ?
                        wait = tracee.wait_one()?;
                    }

                    if let Some(t) = self.tracee_ctl.tracee_mut(tracee.pid) {
                        t.stop();
                    }
                }
            }
        }

        Ok(())
    }

    /// Handle tracee event wired by `wait` syscall.
    /// After this function ends tracee_ctl must be in consistent state.
    /// If debugee process stop detected - returns stop reason.
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
                        self.tracee_ctl.tracee_ensure_mut(pid).stop();
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
                            Some(tracee) => tracee.stop(),
                            None => {
                                self.tracee_ctl.add(pid);
                            }
                        }
                    }
                    libc::PTRACE_EVENT_EXIT => {
                        // Stop the tracee at exit
                        let tracee = self.tracee_ctl.remove(pid);
                        if let Some(tracee) = tracee {
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
                            self.group_stop_interrupt(pid)?;

                            Ok(Some(StopReason::Breakpoint(pid, current_pc)))
                        }
                        code => bail!("unexpected SIGTRAP code {code}"),
                    },
                    _ => {
                        // group-stop from outside
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
