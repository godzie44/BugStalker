use crate::debugger::address::{Address, GlobalAddress, RelocatedAddress};
use crate::debugger::debugee::dwarf::unwind::libunwind;
use crate::debugger::debugee::dwarf::unwind::libunwind::Backtrace;
use crate::debugger::debugee::dwarf::{DebugeeContext, EndianArcSlice};
use crate::debugger::debugee::rendezvous::Rendezvous;
use crate::debugger::debugee::tracee::{Tracee, TraceeCtl};
use crate::debugger::debugee::tracer::{StopReason, TraceContext, Tracer};
use crate::weak_error;
use anyhow::anyhow;
use log::{info, warn};
use nix::unistd::Pid;
use object::{Object, ObjectSection};
use proc_maps::MapRange;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub mod dwarf;
mod rendezvous;
pub mod tracee;
pub mod tracer;

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
    pub thread: Tracee,
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
    pub dwarf: DebugeeContext<EndianArcSlice>,
    /// elf file sections (name => address).
    object_sections: HashMap<String, u64>,
    /// rendezvous struct maintained by dyn linker.
    rendezvous: Option<Rendezvous>,
    /// Debugee tracer. Control debugee process.
    pub tracer: Tracer,
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
            object_sections: object
                .sections()
                .filter_map(|section| Some((section.name().ok()?.to_string(), section.address())))
                .collect(),
            rendezvous: None,
            tracer: Tracer::new(proc),
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
    pub fn rendezvous(&self) -> &Rendezvous {
        self.rendezvous.as_ref().expect("rendezvous must exists")
    }

    fn init_libthread_db(&mut self) {
        match self.tracer.tracee_ctl.init_thread_db() {
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

    pub fn trace_until_stop(&mut self, ctx: TraceContext) -> anyhow::Result<StopReason> {
        let event = self.tracer.resume(ctx)?;
        match event {
            StopReason::DebugeeExit(_) => {
                self.execution_status = ExecutionStatus::Exited;
            }
            StopReason::DebugeeStart => {
                self.execution_status = ExecutionStatus::InProgress;
                self.mapping_addr = Some(self.define_mapping_addr()?);
            }
            StopReason::Breakpoint(tid, addr) => {
                let at_entry_point = ctx
                    .breakpoints
                    .iter()
                    .find(|bp| bp.addr == Address::Relocated(addr))
                    .map(|bp| bp.is_entry_point());
                if at_entry_point == Some(true) {
                    self.rendezvous = Some(Rendezvous::new(
                        tid,
                        self.mapping_offset(),
                        &self.object_sections,
                    )?);
                    self.init_libthread_db();
                }
            }
            _ => {}
        }

        Ok(event)
    }

    pub fn tracee_ctl(&self) -> &TraceeCtl {
        &self.tracer.tracee_ctl
    }

    fn define_mapping_addr(&mut self) -> anyhow::Result<usize> {
        let absolute_debugee_path_buf = self.path.canonicalize()?;
        let absolute_debugee_path = absolute_debugee_path_buf.as_path();

        let proc_maps: Vec<MapRange> =
            proc_maps::get_process_maps(self.tracee_ctl().proc_pid().as_raw())?
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
        let threads = self.tracee_ctl().snapshot();
        Ok(threads
            .into_iter()
            .map(|tracee| {
                let mb_bt = weak_error!(libunwind::unwind(tracee.pid));
                ThreadSnapshot {
                    in_focus: &tracee == self.tracee_ctl().tracee_in_focus(),
                    thread: tracee,
                    bt: mb_bt,
                }
            })
            .collect())
    }

    /// Returns tracee currently in focus.
    pub fn tracee_in_focus(&self) -> &Tracee {
        self.tracer.tracee_ctl.tracee_in_focus()
    }
}
