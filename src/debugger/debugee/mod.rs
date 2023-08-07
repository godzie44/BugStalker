use crate::debugger::address::{GlobalAddress, RelocatedAddress};
use crate::debugger::debugee::dwarf::unwind::libunwind;
use crate::debugger::debugee::dwarf::unwind::libunwind::Backtrace;
use crate::debugger::debugee::dwarf::{DebugeeContext, EndianArcSlice};
use crate::debugger::debugee::registry::DwarfRegistry;
use crate::debugger::debugee::rendezvous::Rendezvous;
use crate::debugger::debugee::tracee::{Tracee, TraceeCtl};
use crate::debugger::debugee::tracer::{StopReason, TraceContext, Tracer};
use crate::debugger::unwind::FrameSpan;
use crate::debugger::ExplorationContext;
use crate::weak_error;
use anyhow::anyhow;
use log::{info, warn};
use nix::unistd::Pid;
use object::{Object, ObjectSection};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub mod dwarf;
mod registry;
mod rendezvous;
pub mod tracee;
pub mod tracer;

/// Stack frame information.
#[derive(Debug, Default, Clone)]
pub struct FrameInfo {
    pub num: u32,
    pub frame: FrameSpan,
    /// Dwarf frame base address
    pub base_addr: RelocatedAddress,
    /// CFA is defined to be the value of the stack  pointer at the call site in the previous frame
    /// (which may be different from its value on entry to the current frame).
    pub cfa: RelocatedAddress,
    pub return_addr: Option<RelocatedAddress>,
}

/// Debugee thread description.
pub struct ThreadSnapshot {
    /// Running thread info - pid, number and status.
    pub thread: Tracee,
    /// Backtrace
    pub bt: Option<Backtrace>,
    /// On focus frame number (if focus on this thread)
    pub focus_frame: Option<usize>,
    /// True if thread in focus, false elsewhere
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
    /// Debugee tracer. Control debugee process.
    tracer: Tracer,
    /// path to debugee file.
    path: PathBuf,
    /// elf file sections (name => address).
    object_sections: HashMap<String, u64>,
    /// rendezvous struct maintained by dyn linker.
    rendezvous: Option<Rendezvous>,
    /// Registry for dwarf information of program and shared libraries.
    dwarf_registry: DwarfRegistry,
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
        let dwarf = dwarf_builder.build(object)?;
        let registry = DwarfRegistry::new(proc, path.to_path_buf(), dwarf);

        Ok(Self {
            execution_status: ExecutionStatus::Unload,
            path: path.into(),
            object_sections: object
                .sections()
                .filter_map(|section| Some((section.name().ok()?.to_string(), section.address())))
                .collect(),
            rendezvous: None,
            tracer: Tracer::new(proc),
            dwarf_registry: registry,
        })
    }

    /// Create new [`Debugee`] with same dwarf context.
    ///
    /// # Arguments
    ///
    /// * `proc`: new process pid.
    pub fn extend(&self, proc: Pid) -> Self {
        Self {
            execution_status: ExecutionStatus::Unload,
            path: self.path.clone(),
            object_sections: self.object_sections.clone(),
            rendezvous: None,
            tracer: Tracer::new(proc),
            dwarf_registry: self.dwarf_registry.extend(proc),
        }
    }

    pub fn in_progress(&self) -> bool {
        self.execution_status == ExecutionStatus::InProgress
    }

    /// Return debugee process mapping offset.
    ///
    /// # Panics
    /// This method will panic if called before debugee started,
    /// calling a method on time is the responsibility of the caller.
    pub fn mapping_offset(&self) -> usize {
        self.dwarf_registry
            .get_program_mapping()
            .expect("mapping address must exists")
    }

    /// Return rendezvous struct.
    ///
    /// # Panics
    /// This method will panic if called before program entry point evaluated,
    /// calling a method on time is the responsibility of the caller.
    pub fn rendezvous(&self) -> &Rendezvous {
        self.rendezvous.as_ref().expect("rendezvous must exists")
    }

    /// Return debugee [`Tracer`]
    pub fn tracer_mut(&mut self) -> &mut Tracer {
        &mut self.tracer
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
                self.dwarf_registry.update_mappings()?;
            }
            StopReason::Breakpoint(tid, addr) => {
                let at_entry_point = ctx
                    .breakpoints
                    .iter()
                    .find(|bp| bp.addr == addr)
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

    #[inline(always)]
    pub fn tracee_ctl(&self) -> &TraceeCtl {
        &self.tracer.tracee_ctl
    }

    pub fn frame_info(&self, ctx: &ExplorationContext) -> anyhow::Result<FrameInfo> {
        let func = self
            .dwarf()
            .find_function_by_pc(ctx.location().global_pc)
            .ok_or_else(|| anyhow!("current function not found"))?;

        let base_addr = func.frame_base_addr(ctx, self)?;
        let cfa = self.dwarf().get_cfa(self, ctx)?;
        let backtrace = libunwind::unwind(ctx.pid_on_focus())?;
        let (bt_frame_num, frame) = backtrace
            .iter()
            .enumerate()
            .find(|(_, frame)| frame.ip == ctx.location().pc)
            .expect("frame must exists");
        let return_addr = backtrace.get(bt_frame_num + 1).map(|f| f.ip);
        Ok(FrameInfo {
            frame: frame.clone(),
            num: bt_frame_num as u32,
            cfa,
            base_addr,
            return_addr,
        })
    }

    pub fn thread_state(&self, ctx: &ExplorationContext) -> anyhow::Result<Vec<ThreadSnapshot>> {
        let threads = self.tracee_ctl().snapshot();
        Ok(threads
            .into_iter()
            .map(|tracee| {
                let mb_bt = weak_error!(libunwind::unwind(tracee.pid));
                let frame_num = mb_bt.as_ref().and_then(|bt| {
                    bt.iter()
                        .enumerate()
                        .find_map(|(i, frame)| (frame.ip == ctx.location().pc).then_some(i))
                });

                ThreadSnapshot {
                    in_focus: tracee.pid == ctx.pid_on_focus(),
                    thread: tracee,
                    bt: mb_bt,
                    focus_frame: frame_num,
                }
            })
            .collect())
    }

    /// Return tracee by it's thread id.
    ///
    /// # Arguments
    ///
    /// * `pid`: tracee thread id
    ///
    /// returns: &Tracee
    ///
    /// # Panics
    ///
    /// This method panics if thread with pid `pid` not run
    pub fn get_tracee_ensure(&self, pid: Pid) -> &Tracee {
        self.tracee_ctl().tracee_ensure(pid)
    }

    /// Return tracee by it's number.
    ///
    /// # Arguments
    ///
    /// * `num`: tracee number
    pub fn get_tracee_by_num(&self, num: u32) -> anyhow::Result<Tracee> {
        let mut snapshot = self.tracee_ctl().snapshot();
        let tracee = snapshot.drain(..).find(|tracee| tracee.number == num);
        tracee.ok_or(anyhow!("tracee {num} not found"))
    }

    #[inline(always)]
    pub fn dwarf(&self) -> &DebugeeContext<EndianArcSlice> {
        self.dwarf_registry.get_program_dwarf()
    }
}
