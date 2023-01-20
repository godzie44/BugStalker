use crate::debugger::debugee::dwarf::{DebugeeContext, EndianRcSlice};
use crate::debugger::debugee::thread::TraceeStatus;
use crate::debugger::debugee_ctl::DebugeeState;
use anyhow::anyhow;
use nix::unistd::Pid;
use proc_maps::MapRange;
use std::fs;
use std::path::{Path, PathBuf};

pub mod dwarf;
pub mod thread;

/// Debugee - represent static and runtime debugee information.
pub struct Debugee {
    /// path to debugee file
    pub path: PathBuf,
    /// debugee process map address
    pub mapping_addr: Option<usize>,
    /// debugee process threads
    pub threads_ctl: thread::ThreadCtl,
    /// preparsed debugee dwarf
    pub dwarf: DebugeeContext<EndianRcSlice>,
}

impl Debugee {
    pub fn new(path: &Path, proc: Pid) -> anyhow::Result<Self> {
        let file = fs::File::open(path)?;

        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        let object = object::File::parse(&*mmap)?;

        let dwarf_builder = dwarf::DebugeeContextBuilder::default();

        Ok(Self {
            path: path.into(),
            mapping_addr: None,
            threads_ctl: thread::ThreadCtl::new(proc),
            dwarf: dwarf_builder.build(&object)?,
        })
    }

    pub fn apply_state(&mut self, state: DebugeeState) -> anyhow::Result<()> {
        match state {
            DebugeeState::ThreadExit(tid) => {
                // at this point thread must already removed from registry
                // anyway `registry.remove` is idempotent
                self.threads_ctl.remove(tid);
            }
            DebugeeState::DebugeeExit(_) => {
                self.threads_ctl.remove(self.threads_ctl.proc_pid());
            }
            DebugeeState::DebugeeStart => {
                self.mapping_addr = Some(self.define_mapping_addr()?);
                self.threads_ctl
                    .set_stop_status(self.threads_ctl.proc_pid());
            }
            DebugeeState::BeforeNewThread(pid, tid) => {
                self.threads_ctl.set_stop_status(pid);
                self.threads_ctl.register(tid);
            }
            DebugeeState::ThreadInterrupt(tid) => {
                if self.threads_ctl.status(tid) == TraceeStatus::Created {
                    self.threads_ctl.set_stop_status(tid);
                    self.threads_ctl.cont_stopped()?;
                } else {
                    self.threads_ctl.set_stop_status(tid);
                }
            }
            DebugeeState::BeforeThreadExit(tid) => {
                self.threads_ctl.set_stop_status(tid);
                self.threads_ctl.cont_stopped()?;
                self.threads_ctl.remove(tid);
            }
            DebugeeState::Breakpoint(tid) => {
                self.threads_ctl.set_thread_to_focus(tid);
                self.threads_ctl.set_stop_status(tid);
                self.threads_ctl.interrupt_running()?;
            }
            DebugeeState::OsSignal(_, tid) => {
                self.threads_ctl.set_thread_to_focus(tid);
                self.threads_ctl.set_stop_status(tid);
                self.threads_ctl.interrupt_running()?;
            }
            _ => {}
        }
        Ok(())
    }

    fn define_mapping_addr(&mut self) -> anyhow::Result<usize> {
        let absolute_debugee_path_buf = self.path.canonicalize()?;
        let absolute_debugee_path = absolute_debugee_path_buf.as_path();

        let proc_maps: Vec<MapRange> =
            proc_maps::get_process_maps(self.threads_ctl.proc_pid().as_raw())?
                .into_iter()
                .filter(|map| map.filename() == Some(absolute_debugee_path))
                .collect();

        let lowest_map = proc_maps
            .iter()
            .min_by(|map1, map2| map1.start().cmp(&map2.start()))
            .ok_or_else(|| anyhow!("mapping not found"))?;

        Ok(lowest_map.start())
    }
}
