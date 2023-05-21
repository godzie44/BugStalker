use crate::debugger::address::RelocatedAddress;
use crate::debugger::debugee::tracee::StopType::Interrupt;
use crate::debugger::debugee::tracee::TraceeStatus::{Running, Stopped};
use crate::debugger::debugee::{Debugee, Location};
use crate::debugger::register::{Register, RegisterMap};
use anyhow::{anyhow, bail};
use itertools::Itertools;
use log::{debug, warn};
use nix::errno::Errno;
use nix::sys;
use nix::sys::signal::Signal;
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::Pid;
use ouroboros::self_referencing;
use std::collections::{HashMap, HashSet};
use thread_db;

#[self_referencing]
struct ThreadDBProcess {
    lib: thread_db::Lib,
    #[borrows(lib)]
    #[covariant]
    process: thread_db::Process<'this>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StopType {
    Interrupt,
    SignalStop(Signal),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TraceeStatus {
    Stopped(StopType),
    Running,
}

/// Tracee is a thread attached to debugger with ptrace.
#[derive(Clone, Debug, PartialEq)]
pub struct Tracee {
    /// Tracee thread id.
    pub pid: Pid,
    /// Tracee current status.
    pub status: TraceeStatus,
}

impl Tracee {
    /// Wait for change of tracee status.
    pub fn wait_one(&self) -> nix::Result<WaitStatus> {
        debug!(target: "tracer", "wait for tracee status, thread {pid}", pid = self.pid);
        let status = waitpid(self.pid, None)?;
        debug!(target: "tracer", "receive tracee status, thread {pid}, status: {status:?}", pid = self.pid);
        Ok(status)
    }

    fn update_status(&mut self, status: TraceeStatus) {
        debug!(
            target: "tracer",
            "tracee accept new status ({status:?}), thread: {pid}",
            pid = self.pid
        );
        self.status = status
    }

    /// Resume tracee with, if signal is some - inject signal or resuming.
    pub fn r#continue(&mut self, sig: Option<Signal>) -> nix::Result<()> {
        debug!(
            target: "tracer",
            "continue tracee execution with signal {sig:?}, thread: {pid}",
            pid = self.pid,
        );

        sys::ptrace::cont(self.pid, sig).map(|ok| {
            self.update_status(Running);
            ok
        })
    }

    /// Set tracee status into stop.
    ///
    /// Note: this function does not actually stop the tracee.
    pub fn set_stop(&mut self, r#type: StopType) {
        self.update_status(Stopped(r#type));
    }

    /// Returns true if tracee in one of stopping statuses.
    pub fn is_stopped(&self) -> bool {
        matches!(self.status, Stopped(_))
    }

    /// Get current program counter value.
    pub fn pc(&self) -> nix::Result<RelocatedAddress> {
        RegisterMap::current(self.pid)
            .map(|reg_map| RelocatedAddress::from(reg_map.value(Register::Rip)))
    }

    /// Set new program counter value.
    pub fn set_pc(&self, value: u64) -> nix::Result<()> {
        let mut map = RegisterMap::current(self.pid)?;
        map.update(Register::Rip, value);
        map.persist(self.pid)
    }

    /// Get current tracee location.
    pub fn location(&self, debugee: &Debugee) -> nix::Result<Location> {
        let pc = self.pc()?;
        Ok(Location {
            pid: self.pid,
            pc,
            global_pc: pc.into_global(debugee.mapping_offset()),
        })
    }
}

pub struct TraceeCtl {
    process_pid: Pid,
    in_tracee_tid: Pid,
    threads_state: HashMap<Pid, Tracee>,
    thread_db_proc: Option<ThreadDBProcess>,
}

impl TraceeCtl {
    pub fn new(proc_pid: Pid) -> TraceeCtl {
        Self {
            process_pid: proc_pid,
            in_tracee_tid: proc_pid,
            threads_state: HashMap::from([(
                proc_pid,
                Tracee {
                    pid: proc_pid,
                    status: Stopped(Interrupt),
                },
            )]),
            thread_db_proc: None,
        }
    }

    pub(crate) fn tracee(&mut self, pid: Pid) -> Option<&Tracee> {
        self.threads_state.get(&pid)
    }

    pub(crate) fn tracee_mut(&mut self, pid: Pid) -> Option<&mut Tracee> {
        self.threads_state.get_mut(&pid)
    }

    pub(crate) fn tracee_ensure(&self, pid: Pid) -> &Tracee {
        self.threads_state.get(&pid).unwrap()
    }

    pub(crate) fn tracee_ensure_mut(&mut self, pid: Pid) -> &mut Tracee {
        self.tracee_mut(pid).unwrap()
    }

    /// Return pid of debugee process main thread.
    pub fn proc_pid(&self) -> Pid {
        self.process_pid
    }

    /// Set tracee into focus.
    pub fn set_tracee_to_focus(&mut self, tid: Pid) {
        self.in_tracee_tid = tid
    }

    /// Return current focused tracee.
    pub(super) fn tracee_in_focus(&self) -> &Tracee {
        &self.threads_state[&self.in_tracee_tid]
    }

    /// Adds thread to badge in `created` status.
    /// `created` actual for ptrace events like PTRACE_EVENT_CLONE, when wee known about new thread but
    /// this not created yet.
    pub fn add(&mut self, pid: Pid) -> &Tracee {
        debug!(target: "tracer", "add new tracee, thread: {pid}");
        let new = Tracee {
            pid,
            status: Stopped(Interrupt),
        };
        self.threads_state.insert(pid, new);
        &self.threads_state[&pid]
    }

    /// Remove thread from budge.
    pub fn remove(&mut self, pid: Pid) -> Option<Tracee> {
        debug!(target: "tracer", "try to remove tracee, thread: {pid}");
        self.threads_state.remove(&pid)
    }

    /// Continue all currently stopped tracees.
    pub fn cont_stopped(&mut self) -> Result<(), anyhow::Error> {
        let mut errors = vec![];

        self.threads_state.iter_mut().for_each(|(_, tracee)| {
            if !tracee.is_stopped() {
                return;
            }

            if let Err(e) = tracee.r#continue(None) {
                // if no such process - continue, it will be removed later, on PTRACE_EVENT_EXIT event.
                if Errno::ESRCH == e {
                    warn!("thread {} not found, ESRCH", tracee.pid);
                    return;
                }

                errors.push(anyhow::Error::from(e).context(format!("thread: {}", tracee.pid)));
            }
        });

        if !errors.is_empty() {
            bail!(errors.into_iter().join(";"))
        }
        Ok(())
    }

    /// Continue all currently stopped tracees.
    ///
    /// # Arguments
    ///
    /// * `inject_request`: send signal to one of threads.
    /// * `exclude`: set of threads that must be not continued.
    pub fn cont_stopped_ex(
        &mut self,
        inject_request: Option<(Pid, Signal)>,
        exclude: HashSet<Pid>,
    ) -> Result<(), anyhow::Error> {
        let mut errors = vec![];
        let (signal, pid) = (inject_request.map(|s| s.1), inject_request.map(|s| s.0));

        self.threads_state.iter_mut().for_each(|(_, tracee)| {
            if exclude.contains(&tracee.pid) {
                return;
            }

            if !tracee.is_stopped() {
                return;
            }

            let resume_sign = if Some(tracee.pid) == pid {
                signal
            } else {
                None
            };

            if let Err(e) = tracee.r#continue(resume_sign) {
                // if no such process - continue, it will be removed later, on PTRACE_EVENT_EXIT event.
                if Errno::ESRCH == e {
                    warn!("thread {} not found, ESRCH", tracee.pid);
                    return;
                }

                errors.push(anyhow::Error::from(e).context(format!("thread: {}", tracee.pid)));
            }
        });

        if !errors.is_empty() {
            bail!(errors.into_iter().join(";"))
        }
        Ok(())
    }

    pub fn snapshot(&self) -> Vec<Tracee> {
        self.threads_state.values().cloned().collect()
    }

    /// Load libthread_db and init libthread_db process handle.
    /// libthread_db must initialized after first thread created.
    pub(super) fn init_thread_db(&mut self) -> anyhow::Result<()> {
        let thread_db_lib = thread_db::Lib::try_load()?;
        let td_process = ThreadDBProcessTryBuilder {
            lib: thread_db_lib,
            process_builder: |lib| lib.attach(self.process_pid),
        }
        .try_build()?;
        self.thread_db_proc = Some(td_process);
        Ok(())
    }

    /// Get address of thread local variable. link_map_addr - address of module link_map struct.
    pub fn tls_addr(
        &self,
        tid: Pid,
        link_map_addr: RelocatedAddress,
        offset: usize,
    ) -> anyhow::Result<RelocatedAddress> {
        let td_proc = self
            .thread_db_proc
            .as_ref()
            .ok_or_else(|| anyhow!("libthread_db not enabled"))?;

        let thread: thread_db::Thread = td_proc.borrow_process().get_thread(tid)?;

        Ok(RelocatedAddress::from(
            thread.tls_addr(link_map_addr.into(), offset)? as usize,
        ))
    }
}
