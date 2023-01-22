use crate::debugger::debugee::dwarf::{DebugeeContext, EndianRcSlice};
use crate::debugger::debugee::thread::TraceeStatus;
use crate::debugger::debugee_ctl::DebugeeState;
use anyhow::anyhow;
use log::{info, warn};
use nix::unistd::Pid;
use proc_maps::MapRange;
use std::path::{Path, PathBuf};
use std::{fs, sync};

pub mod dwarf;
pub mod thread;

static LIBTHREAD_DB_ONCE: sync::Once = sync::Once::new();

/// Debugee - represent static and runtime debugee information.
pub struct Debugee {
    /// true if debugee currently start
    pub in_progress: bool,
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
    pub fn new_non_running(path: &Path, proc: Pid) -> anyhow::Result<Self> {
        let file = fs::File::open(path)?;

        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        let object = object::File::parse(&*mmap)?;

        let dwarf_builder = dwarf::DebugeeContextBuilder::default();

        Ok(Self {
            in_progress: false,
            path: path.into(),
            mapping_addr: None,
            threads_ctl: thread::ThreadCtl::new(proc),
            dwarf: dwarf_builder.build(&object)?,
        })
    }

    /// Return debugee process mapping offset.
    /// This method will panic if called before debugee started,
    /// calling a method on time is the responsibility of the caller.
    pub fn mapping_offset(&self) -> usize {
        self.mapping_addr.expect("mapping address must exists")
    }

    fn init_libthread_db(&mut self) {
        match self.threads_ctl.init_thread_db() {
            Ok(_) => {
                info!("libthread_db enabled")
            }
            Err(e) => {
                warn!(
                    "libthread_db load fail with \"{e}\", some thread debug functions are omitted"
                );
            }
        }
    }

    pub fn apply_state(&mut self, state: DebugeeState) -> anyhow::Result<()> {
        match state {
            DebugeeState::DebugeeStart => {
                self.in_progress = true;
                self.mapping_addr = Some(self.define_mapping_addr()?);
                self.threads_ctl
                    .set_stop_status(self.threads_ctl.proc_pid());
            }
            DebugeeState::DebugeeExit(_) => {
                self.threads_ctl.remove(self.threads_ctl.proc_pid());
            }
            DebugeeState::ThreadExit(tid) => {
                // at this point thread must already removed from registry
                // anyway `registry.remove` is idempotent
                self.threads_ctl.remove(tid);
            }
            DebugeeState::BeforeNewThread(pid, tid) => {
                self.threads_ctl.set_stop_status(pid);
                self.threads_ctl.register(tid);
            }
            DebugeeState::ThreadInterrupt(tid) => {
                if self.threads_ctl.status(tid) == TraceeStatus::Created {
                    LIBTHREAD_DB_ONCE.call_once(|| {
                        self.init_libthread_db();
                    });
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
