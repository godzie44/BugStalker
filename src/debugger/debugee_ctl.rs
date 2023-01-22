use crate::debugger::code;
use crate::debugger::debugee_ctl::DebugeeState::{
    BeforeNewThread, BeforeThreadExit, Breakpoint, DebugeeExit, DebugeeStart, NoSuchProcess,
    OsSignal, ThreadExit, ThreadInterrupt, TrapTrace, UnexpectedPtraceEvent, UnexpectedWaitStatus,
};
use anyhow::bail;
use log::warn;
use nix::errno::Errno;
use nix::libc::{pid_t, siginfo_t};
use nix::sys::signal::Signal;
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::Pid;
use nix::{libc, sys};

/// Debugee state
#[derive(Clone, Copy, Debug)]
pub enum DebugeeState {
    /// Thread exited with tread id
    ThreadExit(Pid),
    /// Whole debugee process exited with code
    DebugeeExit(i32),
    /// Debugee just started
    DebugeeStart,
    /// New thread prepared to starting
    BeforeNewThread(Pid, Pid),
    /// Thread interrupt by ptrace call
    ThreadInterrupt(Pid),
    /// Thread prepared to exit
    BeforeThreadExit(Pid),

    Breakpoint(Pid),

    OsSignal(siginfo_t, Pid),

    UnexpectedPtraceEvent,
    UnexpectedWaitStatus,
    NoSuchProcess,
    TrapTrace,
}

pub struct DebugeeControlFlow {
    proc_id: Pid,
}

impl DebugeeControlFlow {
    pub fn new(proc_pid: Pid) -> Self {
        Self { proc_id: proc_pid }
    }

    pub fn tick(&self) -> anyhow::Result<DebugeeState> {
        let status = waitpid(Pid::from_raw(-1), None)?;

        match status {
            WaitStatus::Exited(pid, code) => {
                if pid == self.proc_id {
                    return Ok(DebugeeExit(code));
                }
                Ok(ThreadExit(pid))
            }
            WaitStatus::PtraceEvent(pid, _, code) => {
                match code {
                    libc::PTRACE_EVENT_EXEC => {
                        // fire just before debugee start
                        // cause currently `fork()` in debugee is unsupported we expect this code calling once
                        Ok(DebugeeStart)
                    }
                    libc::PTRACE_EVENT_CLONE => {
                        // fire just before new thread created
                        let tid = sys::ptrace::getevent(pid)?;
                        Ok(BeforeNewThread(pid, Pid::from_raw(tid as pid_t)))
                    }
                    libc::PTRACE_EVENT_STOP => {
                        // fire right after new thread started or PTRACE_INTERRUPT called.
                        // Also PTRACE_INTERRUPT handle by ::stop_threads.
                        Ok(ThreadInterrupt(pid))
                    }
                    libc::PTRACE_EVENT_EXIT => Ok(BeforeThreadExit(pid)),
                    _ => {
                        warn!("unsupported ptrace event, code: {code}");
                        Ok(UnexpectedPtraceEvent)
                    }
                }
            }

            WaitStatus::Stopped(pid, signal) => {
                let info = match sys::ptrace::getsiginfo(pid) {
                    Ok(info) => info,
                    Err(Errno::ESRCH) => return Ok(NoSuchProcess),
                    Err(e) => return Err(e.into()),
                };

                match signal {
                    Signal::SIGTRAP => match info.si_code {
                        code::TRAP_TRACE => Ok(TrapTrace),
                        code::TRAP_BRKPT | code::SI_KERNEL => Ok(Breakpoint(pid)),
                        code => bail!("unexpected SIGTRAP code {code}"),
                    },
                    _ => Ok(OsSignal(info, pid)),
                }
            }
            _ => {
                warn!("unexpected wait status: {status:?}");
                Ok(UnexpectedWaitStatus)
            }
        }
    }

    pub fn thread_step(tid: Pid) -> anyhow::Result<()> {
        sys::ptrace::step(tid, None)?;
        let _status = waitpid(tid, None)?;
        debug_assert!({
            // assert TRAP_TRACE code
            let info = sys::ptrace::getsiginfo(tid);
            matches!(WaitStatus::Stopped, _status)
                && info
                    .map(|info| info.si_code == code::TRAP_TRACE)
                    .unwrap_or(false)
        });
        Ok(())
    }
}
