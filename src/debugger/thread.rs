use crate::debugger::thread::TraceeStatus::{Created, Running, Stopped};
use anyhow::bail;
use itertools::Itertools;
use log::warn;
use nix::errno::Errno;
use nix::sys;
use nix::unistd::Pid;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum TraceeStatus {
    Created,
    Stopped,
    Running,
    OutOfReach,
}

pub(super) struct Registry {
    debugee_main: Pid,
    in_focus: Cell<Pid>,
    state: RefCell<HashMap<Pid, TraceeStatus>>,
}

impl Registry {
    pub(super) fn new(main_pid: Pid) -> Registry {
        Self {
            debugee_main: main_pid,
            in_focus: Cell::new(main_pid),
            state: RefCell::new(HashMap::from([(main_pid, Stopped)])),
        }
    }

    /// Return pid of debugee process main thread.
    pub(super) fn main_thread(&self) -> Pid {
        self.debugee_main
    }

    /// Set in focus thread.
    pub(super) fn set_in_focus_thread(&self, thread: Pid) {
        self.in_focus.set(thread)
    }

    /// Return current focused thread.
    pub(super) fn on_focus_thread(&self) -> Pid {
        self.in_focus.get()
    }

    /// Adds thread to badge in `created` status.
    /// `created` actual for ptrace events like PTRACE_EVENT_CLONE, when wee known about new thread but
    /// this not created yet.
    pub(super) fn register(&self, pid: Pid) {
        self.state.borrow_mut().insert(pid, Created);
    }

    /// Remove thread from budge.
    pub(super) fn remove(&self, pid: Pid) {
        self.state.borrow_mut().remove(&pid);
    }

    /// Manual set's thread in stop status.
    pub(super) fn set_stop_status(&self, pid: Pid) {
        if let Some(status) = self.state.borrow_mut().get_mut(&pid) {
            *status = Stopped
        }
    }

    /// Continue all currently stopped threads.
    pub(super) fn cont_stopped(&self) -> Result<(), anyhow::Error> {
        let mut errors = vec![];

        self.state
            .borrow_mut()
            .iter_mut()
            .for_each(|(pid, status)| {
                if *status == Stopped {
                    if let Err(e) = sys::ptrace::cont(*pid, None) {
                        // if no such process - continue, it will be removed later, on PTRACE_EVENT_EXIT event.
                        if Errno::ESRCH == e {
                            warn!("thread {pid} not found, ESRCH");
                            return;
                        }

                        errors.push(anyhow::Error::from(e).context(format!("thread: {pid}")));
                    } else {
                        *status = Running
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
    pub(super) fn interrupt_running(&self) -> Result<(), anyhow::Error> {
        let mut errors = vec![];

        self.state
            .borrow_mut()
            .iter_mut()
            .for_each(|(pid, status)| {
                if *status == Running {
                    if let Err(e) = sys::ptrace::interrupt(*pid) {
                        // if no such process - continue, it will be removed later, on PTRACE_EVENT_EXIT event.
                        if Errno::ESRCH == e {
                            warn!("thread {pid} not found, ESRCH");
                            return;
                        }

                        errors.push(anyhow::Error::from(e).context(format!("thread: {pid}")));
                    } else {
                        *status = Stopped
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
            .borrow()
            .get(&pid)
            .cloned()
            .unwrap_or(TraceeStatus::OutOfReach)
    }
}
