use crate::debugger::address::RelocatedAddress;
use crate::debugger::debugee::thread::TraceeStatus::{Created, Running, Stopped};
use anyhow::{anyhow, bail};
use itertools::Itertools;
use log::warn;
use nix::errno::Errno;
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::Pid;
use nix::{libc, sys};
use ouroboros::self_referencing;
use std::collections::HashMap;
use thread_db;

#[self_referencing]
struct ThreadDBProcess {
    lib: thread_db::Lib,
    #[borrows(lib)]
    #[covariant]
    process: thread_db::Process<'this>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TraceeStatus {
    Created,
    Stopped,
    Running,
    OutOfReach,
}

#[derive(Clone, Debug)]
pub struct TraceeThread {
    pub pid: Pid,
    pub status: TraceeStatus,
}

pub struct ThreadCtl {
    process_pid: Pid,
    in_focus_tid: Pid,
    threads_state: HashMap<Pid, TraceeThread>,
    thread_db_proc: Option<ThreadDBProcess>,
}

impl ThreadCtl {
    pub fn new(proc_pid: Pid) -> ThreadCtl {
        Self {
            process_pid: proc_pid,
            in_focus_tid: proc_pid,
            threads_state: HashMap::from([(
                proc_pid,
                TraceeThread {
                    pid: proc_pid,
                    status: Stopped,
                },
            )]),
            thread_db_proc: None,
        }
    }

    /// Return pid of debugee process main thread.
    pub fn proc_pid(&self) -> Pid {
        self.process_pid
    }

    /// Set thread into focus.
    pub fn set_thread_to_focus(&mut self, tid: Pid) {
        self.in_focus_tid = tid
    }

    /// Return current focused thread.
    pub(super) fn thread_in_focus(&self) -> Pid {
        self.in_focus_tid
    }

    /// Adds thread to badge in `created` status.
    /// `created` actual for ptrace events like PTRACE_EVENT_CLONE, when wee known about new thread but
    /// this not created yet.
    pub fn register(&mut self, pid: Pid) {
        let new = TraceeThread {
            pid,
            status: Created,
        };
        self.threads_state.insert(pid, new);
    }

    /// Remove thread from budge.
    pub fn remove(&mut self, pid: Pid) {
        self.threads_state.remove(&pid);
    }

    /// Manual set's thread in stop status.
    pub fn set_stop_status(&mut self, pid: Pid) {
        if let Some(thread) = self.threads_state.get_mut(&pid) {
            thread.status = Stopped
        }
    }

    /// Continue all currently stopped threads.
    pub fn cont_stopped(&mut self) -> Result<(), anyhow::Error> {
        let mut errors = vec![];

        self.threads_state.iter_mut().for_each(|(_, thread)| {
            if thread.status == Stopped {
                if let Err(e) = sys::ptrace::cont(thread.pid, None) {
                    // if no such process - continue, it will be removed later, on PTRACE_EVENT_EXIT event.
                    if Errno::ESRCH == e {
                        warn!("thread {} not found, ESRCH", thread.pid);
                        return;
                    }

                    errors.push(anyhow::Error::from(e).context(format!("thread: {}", thread.pid)));
                } else {
                    thread.status = Running
                }
            }
        });

        if !errors.is_empty() {
            bail!(errors.into_iter().join(";"))
        }
        Ok(())
    }

    /// Interrupt all currently running threads.
    /// PTRACE_EVENT_STOP will happen.
    pub fn interrupt_running(&mut self) -> Result<(), anyhow::Error> {
        let mut errors = vec![];
        let mut assume_stopped = vec![];
        self.threads_state.iter_mut().for_each(|(_, thread)| {
            if thread.status == Running {
                if let Err(e) = sys::ptrace::interrupt(thread.pid) {
                    // if no such process - continue, it will be removed later, on PTRACE_EVENT_EXIT event.
                    if Errno::ESRCH == e {
                        warn!("thread {} not found, ESRCH", thread.pid);
                        return;
                    }

                    errors.push(anyhow::Error::from(e).context(format!("thread: {}", thread.pid)));
                } else {
                    assume_stopped.push(thread.pid);
                }
            }
        });

        for need_assume in assume_stopped {
            match waitpid(need_assume, None) {
                Ok(wait) => {
                    // thread may be exited before stop
                    if matches!(wait, WaitStatus::PtraceEvent(_, _, libc::PTRACE_EVENT_EXIT)) {
                        sys::ptrace::cont(need_assume, None)?;
                        self.remove(need_assume);
                        continue;
                    }

                    debug_assert!(
                        matches!(wait, WaitStatus::PtraceEvent(_, _, libc::PTRACE_EVENT_STOP)),
                        "unexpected sign: {:?}",
                        wait
                    );
                }
                Err(err) => {
                    errors
                        .push(anyhow::Error::from(err).context(format!("thread: {}", need_assume)));
                }
            }

            if let Some(thread) = self.threads_state.get_mut(&need_assume) {
                thread.status = Stopped
            }
        }

        if !errors.is_empty() {
            bail!(errors.into_iter().join(";"))
        }

        Ok(())
    }

    /// Return current thread status.
    /// TraceeStatus::OutOfReach returns if thread not found in budge.
    pub fn status(&self, tid: Pid) -> TraceeStatus {
        self.threads_state
            .get(&tid)
            .map(|t| t.status)
            .unwrap_or(TraceeStatus::OutOfReach)
    }

    pub fn snapshot(&self) -> Vec<TraceeThread> {
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
