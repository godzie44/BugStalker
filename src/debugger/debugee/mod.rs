use crate::debugger::address::{GlobalAddress, RelocatedAddress};
use crate::debugger::debugee::dwarf::unwind::libunwind;
use crate::debugger::debugee::dwarf::unwind::libunwind::Backtrace;
use crate::debugger::debugee::dwarf::{DebugeeContext, EndianRcSlice};
use crate::debugger::debugee::flow::{ControlFlow, DebugeeEvent};
use crate::debugger::debugee::rendezvous::Rendezvous;
use crate::debugger::debugee::thread::{ThreadCtl, TraceeThread};
use crate::weak_error;
use anyhow::anyhow;
use log::{info, warn};
use nix::unistd::Pid;
use object::{Object, ObjectSection};
use proc_maps::MapRange;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub mod dwarf;
pub mod flow;
mod rendezvous;
pub mod thread;

/// Stack frame information.
#[derive(Debug, Default, Clone)]
pub struct FrameInfo {
    pub base_addr: RelocatedAddress,
    /// CFA is defined to be the value of the stack  pointer at the call site in the previous frame
    /// (which may be different from its value on entry to the current frame).
    pub cfa: RelocatedAddress,
    pub return_addr: Option<RelocatedAddress>,
}

pub struct ThreadSnapshot {
    pub thread: TraceeThread,
    pub bt: Option<Backtrace>,
    pub in_focus: bool,
}

/// Thread position.
/// Contains pid of thread, relocated and global address of instruction where thread stop.
#[derive(Clone, Copy, Debug)]
pub struct Location {
    pub pc: RelocatedAddress,
    pub global_pc: GlobalAddress,
    pub pid: Pid,
}

#[derive(PartialEq)]
pub enum ExecutionStatus {
    Unload,
    InProgress,
    Exited,
}

/// Debugee - represent static and runtime debugee information.
pub struct Debugee {
    /// debugee running-status.
    pub execution_status: ExecutionStatus,
    /// path to debugee file.
    pub path: PathBuf,
    /// debugee process map address.
    pub mapping_addr: Option<usize>,
    /// preparsed debugee dwarf.
    pub dwarf: DebugeeContext<EndianRcSlice>,
    /// debugee control flow
    pub control_flow: ControlFlow,
    /// elf file sections (name => address).
    object_sections: HashMap<String, u64>,
    /// rendezvous struct maintained by dyn linker.
    rendezvous: Option<Rendezvous>,
}

impl Debugee {
    pub fn new_non_running<'a, 'b, OBJ>(
        path: &Path,
        proc: Pid,
        object: &'a OBJ,
    ) -> anyhow::Result<Self>
    where
        'a: 'b,
        OBJ: Object<'a, 'b>,
    {
        let dwarf_builder = dwarf::DebugeeContextBuilder::default();
        Ok(Self {
            execution_status: ExecutionStatus::Unload,
            path: path.into(),
            mapping_addr: None,
            dwarf: dwarf_builder.build(object)?,
            control_flow: ControlFlow::new(proc, GlobalAddress::from(object.entry() as usize)),
            object_sections: object
                .sections()
                .filter_map(|section| Some((section.name().ok()?.to_string(), section.address())))
                .collect(),
            rendezvous: None,
        })
    }

    /// Return debugee process mapping offset.
    /// This method will panic if called before debugee started,
    /// calling a method on time is the responsibility of the caller.
    pub fn mapping_offset(&self) -> usize {
        self.mapping_addr.expect("mapping address must exists")
    }

    /// Return rendezvous struct.
    /// This method will panic if called before program entry point evaluated,
    /// calling a method on time is the responsibility of the caller.
    #[allow(unused)]
    pub fn rendezvous(&self) -> &Rendezvous {
        self.rendezvous.as_ref().expect("rendezvous must exists")
    }

    fn init_libthread_db(&mut self) {
        match self.control_flow.threads_ctl.init_thread_db() {
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

    pub fn control_flow_tick(&mut self) -> anyhow::Result<DebugeeEvent> {
        let event = self.control_flow.tick(self.mapping_addr)?;
        match event {
            DebugeeEvent::DebugeeExit(_) => {
                self.execution_status = ExecutionStatus::Exited;
            }
            DebugeeEvent::DebugeeStart => {
                self.execution_status = ExecutionStatus::InProgress;
                self.mapping_addr = Some(self.define_mapping_addr()?);
            }
            flow::DebugeeEvent::AtEntryPoint(tid) => {
                self.rendezvous = Some(Rendezvous::new(
                    tid,
                    self.mapping_offset(),
                    &self.object_sections,
                )?);
                self.init_libthread_db();
            }
            _ => {}
        }

        Ok(event)
    }

    pub fn threads_ctl(&self) -> &ThreadCtl {
        &self.control_flow.threads_ctl
    }

    fn define_mapping_addr(&mut self) -> anyhow::Result<usize> {
        let absolute_debugee_path_buf = self.path.canonicalize()?;
        let absolute_debugee_path = absolute_debugee_path_buf.as_path();

        let proc_maps: Vec<MapRange> =
            proc_maps::get_process_maps(self.threads_ctl().proc_pid().as_raw())?
                .into_iter()
                .filter(|map| map.filename() == Some(absolute_debugee_path))
                .collect();

        let lowest_map = proc_maps
            .iter()
            .min_by(|map1, map2| map1.start().cmp(&map2.start()))
            .ok_or_else(|| anyhow!("mapping not found"))?;

        Ok(lowest_map.start())
    }

    pub fn frame_info(&self, location: Location) -> anyhow::Result<FrameInfo> {
        let func = self
            .dwarf
            .find_function_by_pc(location.global_pc)
            .ok_or_else(|| anyhow!("current function not found"))?;

        let base_addr = func.frame_base_addr(location.pid, self, location.global_pc)?;

        let cfa = self.dwarf.get_cfa(self, location)?;
        Ok(FrameInfo {
            cfa,
            base_addr,
            return_addr: libunwind::return_addr(location.pid)?,
        })
    }

    pub fn thread_state(&self) -> anyhow::Result<Vec<ThreadSnapshot>> {
        let threads = self.threads_ctl().snapshot();
        Ok(threads
            .into_iter()
            .map(|thread| {
                let mb_bt = weak_error!(libunwind::unwind(thread.pid));
                ThreadSnapshot {
                    in_focus: thread.pid == self.threads_ctl().thread_in_focus(),
                    thread,
                    bt: mb_bt,
                }
            })
            .collect())
    }

    pub fn thread_in_focus(&self) -> Pid {
        self.threads_ctl().thread_in_focus()
    }

    pub fn current_thread_stop_at(&self) -> nix::Result<Location> {
        self.thread_stop_at(self.threads_ctl().thread_in_focus())
    }

    pub fn thread_stop_at(&self, tid: Pid) -> nix::Result<Location> {
        let pc = self.control_flow.thread_pc(tid)?;
        Ok(Location {
            pid: tid,
            pc,
            global_pc: pc.into_global(self.mapping_offset()),
        })
    }
}
