use crate::debugger::address::{GlobalAddress, RelocatedAddress};
use crate::debugger::code;
use crate::debugger::debugee::thread::{ThreadCtl, TraceeStatus};
use crate::debugger::register::{Register, RegisterMap};
use anyhow::bail;
use log::warn;
use nix::errno::Errno;
use nix::libc::{pid_t, siginfo_t};
use nix::sys::signal::Signal;
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::Pid;
use nix::{libc, sys};

#[derive(Clone, Copy, Debug)]
pub enum DebugeeEvent {
    /// Whole debugee process exited with code
    DebugeeExit(i32),
    /// Debugee just started
    DebugeeStart,
    /// SIGTRAP trace fired
    TrapTrace,
    /// Debugee stopped at program entry point
    AtEntryPoint(Pid),
    /// Debugee stopped at breakpoint
    Breakpoint(Pid, RelocatedAddress),
    /// Debugee stopped with Errno::ESRCH
    NoSuchProcess(Pid),
    /// Debugee stopped with OS signal
    OsSignal(siginfo_t, Pid),
}

pub struct ControlFlow {
    /// debugee entry point address.
    program_ep: GlobalAddress,
    /// debugee process threads.
    pub(super) threads_ctl: ThreadCtl,
}

impl ControlFlow {
    pub fn new(proc_pid: Pid, program_ep: GlobalAddress) -> Self {
        Self {
            program_ep,
            threads_ctl: ThreadCtl::new(proc_pid),
        }
    }

    pub fn tick(&mut self, mapping_offset: Option<usize>) -> anyhow::Result<DebugeeEvent> {
        loop {
            self.threads_ctl.cont_stopped()?;

            let status = waitpid(Pid::from_raw(-1), None)?;

            match status {
                WaitStatus::Exited(pid, code) => {
                    if pid == self.threads_ctl.proc_pid() {
                        self.threads_ctl.remove(self.threads_ctl.proc_pid());
                        return Ok(DebugeeEvent::DebugeeExit(code));
                    }
                    // Thread exited with tread id
                    self.threads_ctl.remove(pid);
                }
                WaitStatus::PtraceEvent(pid, _, code) => {
                    match code {
                        libc::PTRACE_EVENT_EXEC => {
                            // fire just before debugee start
                            // cause currently `fork()` in debugee is unsupported we expect this code calling once
                            self.threads_ctl
                                .set_stop_status(self.threads_ctl.proc_pid());
                            return Ok(DebugeeEvent::DebugeeStart);
                        }
                        libc::PTRACE_EVENT_CLONE => {
                            // fire just before new thread created
                            let tid = Pid::from_raw(sys::ptrace::getevent(pid)? as pid_t);
                            self.threads_ctl.set_stop_status(pid);
                            self.threads_ctl.register(tid);
                        }
                        libc::PTRACE_EVENT_STOP => {
                            // fire right after new thread started or PTRACE_INTERRUPT called.
                            // Also PTRACE_INTERRUPT handle by ::stop_threads.
                            if self.threads_ctl.status(pid) == TraceeStatus::Created {
                                self.threads_ctl.set_stop_status(pid);
                                self.threads_ctl.cont_stopped()?;
                            } else {
                                self.threads_ctl.set_stop_status(pid);
                            }
                        }
                        libc::PTRACE_EVENT_EXIT => {
                            // Stop the tracee at exit
                            self.threads_ctl.set_stop_status(pid);
                            self.threads_ctl.cont_stopped()?;
                            self.threads_ctl.remove(pid);
                        }
                        _ => {
                            warn!("unsupported ptrace event, code: {code}");
                        }
                    }
                }

                WaitStatus::Stopped(pid, signal) => {
                    let info = match sys::ptrace::getsiginfo(pid) {
                        Ok(info) => info,
                        Err(Errno::ESRCH) => return Ok(DebugeeEvent::NoSuchProcess(pid)),
                        Err(e) => return Err(e.into()),
                    };

                    return match signal {
                        Signal::SIGTRAP => match info.si_code {
                            code::TRAP_TRACE => Ok(DebugeeEvent::TrapTrace),
                            code::TRAP_BRKPT | code::SI_KERNEL => {
                                self.threads_ctl.set_thread_to_focus(pid);
                                self.threads_ctl.set_stop_status(pid);
                                self.threads_ctl.interrupt_running()?;

                                self.set_thread_pc(pid, u64::from(self.thread_pc(pid)?) - 1)?;
                                let current_pc = self.thread_pc(pid)?;
                                let offset_pc = current_pc.into_global(mapping_offset.unwrap());
                                if offset_pc == self.program_ep {
                                    Ok(DebugeeEvent::AtEntryPoint(pid))
                                } else {
                                    Ok(DebugeeEvent::Breakpoint(pid, current_pc))
                                }
                            }
                            code => bail!("unexpected SIGTRAP code {code}"),
                        },
                        _ => {
                            self.threads_ctl.set_thread_to_focus(pid);
                            self.threads_ctl.set_stop_status(pid);
                            self.threads_ctl.interrupt_running()?;

                            Ok(DebugeeEvent::OsSignal(info, pid))
                        }
                    };
                }
                _ => {
                    warn!("unexpected wait status: {status:?}");
                }
            };
        }
    }

    pub fn thread_pc(&self, tid: Pid) -> nix::Result<RelocatedAddress> {
        RegisterMap::current(tid)
            .map(|reg_map| RelocatedAddress::from(reg_map.value(Register::Rip)))
    }

    fn set_thread_pc(&self, tid: Pid, value: u64) -> nix::Result<()> {
        let mut map = RegisterMap::current(tid)?;
        map.update(Register::Rip, value);
        map.persist(tid)
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
