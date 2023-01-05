use crate::debugger::thread::TraceeStatus::{Created, Running, Stopped};
use anyhow::bail;
use itertools::Itertools;
use log::warn;
use nix::errno::Errno;
use nix::sys;
use nix::unistd::Pid;
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TraceeStatus {
    Created,
    Stopped,
    Running,
    OutOfReach,
}

#[derive(Clone)]
pub struct TraceeThread {
    pub num: u64,
    pub pid: Pid,
    pub status: TraceeStatus,
}

pub(super) struct Registry {
    debugee_main: Pid,
    in_focus: Pid,
    state: HashMap<Pid, TraceeThread>,
    last_thread_num: u64,
}

impl Registry {
    pub(super) fn new(main_pid: Pid) -> Registry {
        Self {
            debugee_main: main_pid,
            in_focus: main_pid,
            state: HashMap::from([(
                main_pid,
                TraceeThread {
                    num: 1,
                    pid: main_pid,
                    status: Stopped,
                },
            )]),
            last_thread_num: 1,
        }
    }

    /// Return pid of debugee process main thread.
    pub(super) fn main_thread(&self) -> Pid {
        self.debugee_main
    }

    /// Set in focus thread.
    pub(super) fn set_in_focus_thread(&mut self, thread_pid: Pid) {
        self.in_focus = thread_pid
    }

    /// Return current focused thread.
    pub(super) fn on_focus_thread(&self) -> Pid {
        self.in_focus
    }

    /// Adds thread to badge in `created` status.
    /// `created` actual for ptrace events like PTRACE_EVENT_CLONE, when wee known about new thread but
    /// this not created yet.
    pub(super) fn register(&mut self, pid: Pid) {
        self.last_thread_num += 1;
        let new = TraceeThread {
            num: self.last_thread_num,
            pid,
            status: Created,
        };
        self.state.insert(pid, new);
    }

    /// Remove thread from budge.
    pub(super) fn remove(&mut self, pid: Pid) {
        self.state.remove(&pid);
    }

    /// Manual set's thread in stop status.
    pub(super) fn set_stop_status(&mut self, pid: Pid) {
        if let Some(thread) = self.state.get_mut(&pid) {
            thread.status = Stopped
        }
    }

    /// Continue all currently stopped threads.
    pub(super) fn cont_stopped(&mut self) -> Result<(), anyhow::Error> {
        let mut errors = vec![];

        self.state.iter_mut().for_each(|(_, thread)| {
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
    pub(super) fn interrupt_running(&mut self) -> Result<(), anyhow::Error> {
        let mut errors = vec![];

        self.state.iter_mut().for_each(|(_, thread)| {
            if thread.status == Running {
                if let Err(e) = sys::ptrace::interrupt(thread.pid) {
                    // if no such process - continue, it will be removed later, on PTRACE_EVENT_EXIT event.
                    if Errno::ESRCH == e {
                        warn!("thread {} not found, ESRCH", thread.pid);
                        return;
                    }

                    errors.push(anyhow::Error::from(e).context(format!("thread: {}", thread.pid)));
                } else {
                    thread.status = Stopped
                }
            }
        });

        if !errors.is_empty() {
            bail!(errors.into_iter().join(";"))
        }
        Ok(())
    }

    /// Return current thread status.
    /// TraceeStatus::OutOfReach returns if thread not found in budge.
    pub(super) fn status(&self, pid: Pid) -> TraceeStatus {
        self.state
            .get(&pid)
            .map(|t| t.status)
            .unwrap_or(TraceeStatus::OutOfReach)
    }

    pub fn dump(&self) -> Vec<TraceeThread> {
        self.state.iter().map(|(_, v)| v.clone()).collect()
    }
}
