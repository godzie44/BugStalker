use crate::debugger::address::{GlobalAddress, RelocatedAddress};
use crate::debugger::debugee::dwarf::unwind;
use crate::debugger::debugee::dwarf::unwind::Backtrace;
use crate::debugger::debugee::dwarf::DebugInformation;
use crate::debugger::debugee::registry::DwarfRegistry;
use crate::debugger::debugee::rendezvous::Rendezvous;
use crate::debugger::debugee::tracee::{Tracee, TraceeCtl};
use crate::debugger::debugee::tracer::{StopReason, TraceContext, Tracer};
use crate::debugger::register::DwarfRegisterMap;
use crate::debugger::unwind::FrameSpan;
use crate::debugger::ExplorationContext;
use crate::{muted_error, print_warns, weak_error};
use anyhow::anyhow;
use log::{info, warn};
use nix::unistd::Pid;
use nix::NixPath;
use object::{Object, ObjectSection};
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub mod dwarf;
mod ldd;
mod registry;
mod rendezvous;
pub mod tracee;
pub mod tracer;
pub use registry::RegionInfo;

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

#[derive(PartialEq, Clone, Copy)]
pub enum ExecutionStatus {
    Unload,
    InProgress,
    Exited,
}

/// Debugee - represent static and runtime debugee information.
pub struct Debugee {
    /// debugee running-status.
    execution_status: ExecutionStatus,
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
    pub fn new_non_running(path: &Path, proc: Pid, object: &object::File) -> anyhow::Result<Self> {
        let dwarf_builder = dwarf::DebugInformationBuilder::default();
        let dwarf = dwarf_builder.build(path, object)?;
        let mut registry = DwarfRegistry::new(proc, path.to_path_buf(), dwarf);

        // its ok if parse ldd output fail - shared libs will be parsed later - at rendezvous point
        let deps = muted_error!(
            ldd::find_dependencies(path),
            "unsuccessful attempt to use ldd"
        );
        parse_dependencies_into_registry(&mut registry, deps.unwrap_or_default().into_iter());

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

    pub fn execution_status(&self) -> ExecutionStatus {
        self.execution_status
    }

    /// Return true if debugging process in progress
    pub fn is_in_progress(&self) -> bool {
        self.execution_status == ExecutionStatus::InProgress
    }

    /// Return true if debugging process ends
    pub fn is_exited(&self) -> bool {
        self.execution_status == ExecutionStatus::Exited
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
                info!(target: "loading", "libthread_db enabled")
            }
            Err(e) => {
                warn!(
                    target: "loading", "libthread_db load fail with \"{e}\", some thread debug functions are omitted"
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
                print_warns!(self.dwarf_registry.update_mappings(true)?);
            }
            StopReason::Breakpoint(tid, addr) => {
                let at_entry_point = ctx
                    .breakpoints
                    .iter()
                    .find(|bp| bp.addr == addr)
                    .map(|bp| bp.is_entry_point());
                let main_dwarf = self.program_debug_info()?;
                if at_entry_point == Some(true) {
                    self.rendezvous = Some(Rendezvous::new(
                        tid,
                        self.mapping_offset_for_file(main_dwarf)?,
                        &self.object_sections,
                    )?);
                    self.init_libthread_db();

                    let lmaps = self
                        .rendezvous
                        .as_ref()
                        .expect("rendezvous must exists")
                        .link_maps()?;

                    let dep_names: Vec<_> = lmaps
                        .into_iter()
                        .map(|lm| lm.name)
                        .filter(|dep| !self.dwarf_registry.contains(dep))
                        .collect();
                    parse_dependencies_into_registry(
                        &mut self.dwarf_registry,
                        dep_names.into_iter(),
                    );

                    print_warns!(self.dwarf_registry.update_mappings(false)?);
                }
            }
            StopReason::NoSuchProcess(_) => {
                self.execution_status = ExecutionStatus::Exited;
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
        let dwarf = self.debug_info(ctx.location().pc)?;
        let func = dwarf
            .find_function_by_pc(ctx.location().global_pc)?
            .ok_or_else(|| anyhow!("current function not found"))?;

        let base_addr = func.frame_base_addr(ctx, self)?;
        let cfa = dwarf.get_cfa(self, ctx)?;
        let backtrace = self.unwind(ctx.pid_on_focus())?;
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
            .filter_map(|tracee| {
                let _tracee_ctx;
                let tracee_ctx = if tracee.pid == ctx.pid_on_focus() {
                    ctx
                } else {
                    let location = weak_error!(tracee.location(self))?;
                    _tracee_ctx = ExplorationContext::new(location, 0);
                    &_tracee_ctx
                };

                let mb_bt = weak_error!(self.unwind(tracee_ctx.pid_on_focus()));
                let frame_num = mb_bt.as_ref().and_then(|bt| {
                    bt.iter()
                        .enumerate()
                        .find_map(|(i, frame)| (frame.ip == ctx.location().pc).then_some(i))
                });

                Some(ThreadSnapshot {
                    in_focus: tracee.pid == ctx.pid_on_focus(),
                    thread: tracee,
                    bt: mb_bt,
                    focus_frame: frame_num,
                })
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

    /// Return debug information about program determined by program counter address.
    #[inline(always)]
    pub fn debug_info(&self, addr: RelocatedAddress) -> anyhow::Result<&DebugInformation> {
        self.dwarf_registry
            .find_by_addr(addr)
            .ok_or(anyhow!("no debugee information for current location"))
    }

    /// Return debug information about program determined by file which from it been parsed.
    #[inline(always)]
    pub fn debug_info_from_file(&self, path: &Path) -> anyhow::Result<&DebugInformation> {
        self.dwarf_registry
            .find_by_file(path)
            .ok_or(anyhow!("no debugee information for file"))
    }

    /// Get main executable object debug information.
    #[inline(always)]
    pub fn program_debug_info(&self) -> anyhow::Result<&DebugInformation> {
        self.dwarf_registry
            .find_main_program_dwarf()
            .ok_or(anyhow!("no debugee information for executable object"))
    }

    /// Return all known debug information. Debug info about main executable is located at the zero index.
    /// Other information ordered from less compilation unit count to greatest.
    #[inline(always)]
    pub fn debug_info_all(&self) -> Vec<&DebugInformation> {
        self.dwarf_registry.all_dwarf()
    }

    /// Return mapped memory region offset for region.
    ///
    /// # Arguments
    ///
    /// * `pc`: VAS address, determine region for which offset is needed.
    pub fn mapping_offset_for_pc(&self, addr: RelocatedAddress) -> anyhow::Result<usize> {
        self.dwarf_registry.find_mapping_offset(addr).ok_or(anyhow!(
            "determine mapping offset fail, unknown current location"
        ))
    }

    /// Return mapped memory region offset for region.
    ///
    /// # Arguments
    ///
    /// * `dwarf`: debug information (with file path inside) for determine memory region.
    pub fn mapping_offset_for_file(&self, dwarf: &DebugInformation) -> anyhow::Result<usize> {
        self.dwarf_registry
            .find_mapping_offset_for_file(dwarf)
            .ok_or(anyhow!("determine mapping offset fail: unknown segment"))
    }

    /// Unwind debugee thread stack and return a backtrace.
    ///
    /// # Arguments
    ///
    /// * `pid`: thread for unwinding
    pub fn unwind(&self, pid: Pid) -> anyhow::Result<Backtrace> {
        unwind::unwind(self, pid)
    }

    /// Restore registers at chosen frame.
    ///
    /// # Arguments
    ///
    /// * `pid`: thread for unwinding
    /// * `registers`: initial registers state at frame 0 (current frame), will be updated with new values
    /// * `frame_num`: frame number for which registers is restored
    #[allow(unused)]
    pub fn restore_registers_at_frame(
        &self,
        pid: Pid,
        registers: &mut DwarfRegisterMap,
        frame_num: u32,
    ) -> anyhow::Result<()> {
        unwind::restore_registers_at_frame(self, pid, registers, frame_num)
    }

    /// Return return address for thread current program counter.
    ///
    /// # Arguments
    ///
    /// * `pid`: thread for unwinding
    #[allow(unused)]
    pub fn return_addr(&self, pid: Pid) -> anyhow::Result<Option<RelocatedAddress>> {
        unwind::return_addr(self, pid)
    }

    /// Return a ordered list of mapped regions (main executable region at first place).
    pub fn dump_mapped_regions(&self) -> Vec<RegionInfo> {
        self.dwarf_registry.dump()
    }
}

/// Parse dwarf information from new dependency.
fn parse_dependency(dep_file: impl Into<PathBuf>) -> anyhow::Result<Option<DebugInformation>> {
    let dep_file = dep_file.into();

    // empty string represent a program executable that must already parsed
    // libvdso should also be skipped
    if dep_file.is_empty() || dep_file.to_string_lossy().contains("vdso") {
        return Ok(None);
    }

    let file = fs::File::open(&dep_file)?;
    let mmap = unsafe { memmap2::Mmap::map(&file)? };
    let object = object::File::parse(&*mmap)?;

    let dwarf_builder = dwarf::DebugInformationBuilder::default();
    let dwarf = dwarf_builder.build(dep_file.as_path(), &object)?;
    Ok(Some(dwarf))
}

/// Parse list of dependencies and add result into debug information registry.
fn parse_dependencies_into_registry(
    registry: &mut DwarfRegistry,
    deps: impl Iterator<Item = impl Into<PathBuf>>,
) {
    let dwarfs: Vec<_> = deps
        .map(|dep| dep.into())
        .collect::<Vec<_>>()
        .into_par_iter()
        .filter_map(|dep| {
            let parse_result = parse_dependency(&dep);
            match parse_result {
                Ok(mb_dep) => mb_dep.map(|dwarf| (dep, dwarf)),
                Err(e) => {
                    warn!(target: "debugger", "broken dependency {:?}: {:#}", dep, e);
                    None
                }
            }
        })
        .collect();

    dwarfs.into_iter().for_each(|(dep_name, dwarf)| {
        if let Err(e) = registry.add(&dep_name, dwarf) {
            warn!(target: "debugger", "broken dependency {:?}: {:#}", dep_name, e);
        }
    });
}
