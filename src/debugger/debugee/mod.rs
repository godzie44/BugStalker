mod disasm;
pub mod dwarf;
mod ldd;
mod registry;
mod rendezvous;
pub mod tracee;
pub mod tracer;

pub use registry::RegionInfo;
pub use rendezvous::RendezvousError;

use super::r#async::extract_tokio_version_naive;
use super::r#async::TokioVersion;
use crate::debugger::address::{GlobalAddress, RelocatedAddress};
use crate::debugger::breakpoint::{Breakpoint, BrkptType};
use crate::debugger::debugee::disasm::Disassembler;
use crate::debugger::debugee::dwarf::DebugInformation;
use crate::debugger::debugee::dwarf::unit::PlaceDescriptorOwned;
use crate::debugger::debugee::dwarf::unwind;
use crate::debugger::debugee::dwarf::unwind::Backtrace;
use crate::debugger::debugee::registry::DwarfRegistry;
use crate::debugger::debugee::rendezvous::Rendezvous;
use crate::debugger::debugee::tracee::{Tracee, TraceeCtl};
use crate::debugger::debugee::tracer::{StopReason, TraceContext, Tracer};
use crate::debugger::error::Error;
use crate::debugger::error::Error::{FunctionNotFound, MappingOffsetNotFound, TraceeNotFound};
use crate::debugger::process::{Child, Installed};
use crate::debugger::register::DwarfRegisterMap;
use crate::debugger::unwind::FrameSpan;
use crate::debugger::{ExplorationContext, PlaceDescriptor};
use crate::{muted_error, print_warns, weak_error};
use log::{info, warn};
use nix::NixPath;
use nix::unistd::Pid;
use object::{Object, ObjectSection};
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
#[derive(Debug, Clone)]
pub struct ThreadSnapshot {
    /// Running thread info - pid, number and status.
    pub thread: Tracee,
    /// Backtrace
    pub bt: Option<Backtrace>,
    /// Place in source code where thread is stopped
    pub place: Option<PlaceDescriptorOwned>,
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

impl Location {
    pub fn new(pc: RelocatedAddress, global_pc: GlobalAddress, pid: Pid) -> Self {
        Self { pc, global_pc, pid }
    }
}

#[derive(PartialEq, Clone, Copy)]
pub enum ExecutionStatus {
    Unload,
    InProgress,
    Exited,
}

pub struct FunctionAssembly {
    pub name: Option<String>,
    pub addr_in_focus: GlobalAddress,
    pub instructions: Vec<disasm::Instruction>,
}

pub struct FunctionRange<'a> {
    pub name: Option<String>,
    pub stop_place: PlaceDescriptor<'a>,
    pub file: &'a Path,
    pub start_line: u64,
    pub end_line: u64,
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
    /// Disassembler component.
    disassembly: Disassembler,
    /// Loaded libthread_db.
    libthread_db: Arc<thread_db::Lib>,
    /// Version of tokio runtime, if exist.
    tokio_version: Option<TokioVersion>,
}

impl Debugee {
    pub fn new_non_running(
        path: &Path,
        process: &Child<Installed>,
        object: &object::File,
    ) -> Result<Self, Error> {
        let dwarf_builder = dwarf::DebugInformationBuilder;
        let dwarf = dwarf_builder.build(path, object)?;
        let mut registry = DwarfRegistry::new(process.pid(), path.to_path_buf(), dwarf);

        // it is ok if parse ldd output fail -
        // shared libs will be parsed later - at rendezvous point
        let deps = muted_error!(
            ldd::find_dependencies(path),
            "unsuccessful attempt to use ldd"
        );
        parse_dependencies_into_registry(&mut registry, deps.unwrap_or_default().into_iter(), true);

        let tokio_ver: Option<TokioVersion> = object
            .section_by_name(".rodata")
            .and_then(|sect| sect.data().ok())
            .and_then(extract_tokio_version_naive);

        Ok(Self {
            execution_status: ExecutionStatus::Unload,
            path: path.into(),
            object_sections: object
                .sections()
                .filter_map(|section| Some((section.name().ok()?.to_string(), section.address())))
                .collect(),
            rendezvous: None,
            tracer: Tracer::new(process.pid()),
            dwarf_registry: registry,
            disassembly: Disassembler::new()?,
            libthread_db: Arc::new(thread_db::Lib::try_load()?),
            tokio_version: tokio_ver,
        })
    }

    pub fn new_from_external_process(
        path: &Path,
        process: &Child<Installed>,
        object: &object::File,
    ) -> Result<Self, Error> {
        let dwarf_builder = dwarf::DebugInformationBuilder;
        let dwarf = dwarf_builder.build(path, object)?;
        let mut registry = DwarfRegistry::new(process.pid(), path.to_path_buf(), dwarf);
        registry.update_mappings(false)?;

        let main_dwarf = registry
            .find_main_program_dwarf()
            .ok_or(Error::NoDebugInformation("executable object"))?;
        let main_dwarf_offset = registry
            .find_mapping_offset_for_file(main_dwarf)
            .ok_or(MappingOffsetNotFound("unknown segment"))?;
        let object_sections = object
            .sections()
            .filter_map(|section| Some((section.name().ok()?.to_string(), section.address())))
            .collect();
        let tokio_ver: Option<TokioVersion> = object
            .section_by_name(".rodata")
            .and_then(|sect| sect.data().ok())
            .and_then(extract_tokio_version_naive);

        let mut debugee = Self {
            execution_status: ExecutionStatus::InProgress,
            path: path.into(),
            rendezvous: Some(Rendezvous::new(
                process.pid(),
                main_dwarf_offset,
                &object_sections,
            )?),
            object_sections,
            tracer: Tracer::new_external(
                process.pid(),
                &process
                    .external_info()
                    .expect("process is not external")
                    .threads,
            ),
            dwarf_registry: registry,
            disassembly: Disassembler::new()?,
            libthread_db: Arc::new(thread_db::Lib::try_load()?),
            tokio_version: tokio_ver,
        };

        debugee.attach_libthread_db();
        debugee.update_debug_info_registry(true)?;

        Ok(debugee)
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
            disassembly: Disassembler::new().expect("infallible"),
            libthread_db: self.libthread_db.clone(),
            tokio_version: self.tokio_version,
        }
    }

    pub fn tokio_version(&self) -> Option<TokioVersion> {
        self.tokio_version
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

    /// Attach libthread_db to this debugee.
    fn attach_libthread_db(&mut self) {
        let tracee_ctl = &mut self.tracer.tracee_ctl;
        match tracee_ctl.attach_thread_db(self.libthread_db.clone()) {
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

    pub fn trace_until_stop(&mut self, ctx: TraceContext) -> Result<StopReason, Error> {
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
                let mb_brkpt = ctx.breakpoints.iter().find(|bp| bp.addr == addr);
                let mb_type = mb_brkpt.map(|brkpt| brkpt.r#type());

                match mb_type {
                    Some(BrkptType::EntryPoint) => {
                        let main_dwarf = self.program_debug_info()?;
                        self.rendezvous = Some(Rendezvous::new(
                            tid,
                            self.mapping_offset_for_file(main_dwarf)?,
                            &self.object_sections,
                        )?);
                        self.attach_libthread_db();
                        self.update_debug_info_registry(true)?;
                    }
                    Some(BrkptType::LinkerMapFn) => {
                        self.update_debug_info_registry(false)?;
                    }
                    _ => {}
                }
            }
            StopReason::NoSuchProcess(_) => {
                self.execution_status = ExecutionStatus::Exited;
            }
            _ => {}
        }

        Ok(event)
    }

    /// Update all debug information known by debugger. Fetch libraries from rendezvous structures.
    ///
    /// # Arguments
    ///
    /// * `quite`: true for enable logging of library names
    fn update_debug_info_registry(&mut self, quite: bool) -> Result<(), Error> {
        let lmaps = self.rendezvous().link_maps()?;
        let current_deps = lmaps
            .into_iter()
            .map(|lm| PathBuf::from(&lm.name))
            .collect();

        let reload_plan = self.dwarf_registry.reload_plan(current_deps);

        for lib_to_del in reload_plan.to_del {
            self.dwarf_registry.remove(lib_to_del);
        }

        parse_dependencies_into_registry(
            &mut self.dwarf_registry,
            reload_plan.to_add.into_iter(),
            quite,
        );

        print_warns!(self.dwarf_registry.update_mappings(false)?);
        Ok(())
    }

    #[inline(always)]
    pub fn tracee_ctl(&self) -> &TraceeCtl {
        &self.tracer.tracee_ctl
    }

    pub fn frame_info(&self, ctx: &ExplorationContext) -> Result<FrameInfo, Error> {
        let dwarf = self.debug_info(ctx.location().pc)?;
        let func = dwarf
            .find_function_by_pc(ctx.location().global_pc)?
            .ok_or(FunctionNotFound(ctx.location().global_pc))?;

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

    pub fn thread_state(&self, ctx: &ExplorationContext) -> Result<Vec<ThreadSnapshot>, Error> {
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

                let place = mb_bt.as_ref().and_then(|bt| {
                    bt.first().and_then(|first_frame| {
                        let debug_info = self.debug_info(first_frame.ip).ok()?;
                        debug_info
                            .find_place_from_pc(first_frame.ip.into_global(self).ok()?)
                            .ok()?
                    })
                });

                Some(ThreadSnapshot {
                    in_focus: tracee.pid == ctx.pid_on_focus(),
                    thread: tracee,
                    bt: mb_bt,
                    place: place.map(|p| p.to_owned()),
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
    /// This method panics if thread with pid `pid` not runs.
    pub fn get_tracee_ensure(&self, pid: Pid) -> &Tracee {
        self.tracee_ctl().tracee_ensure(pid)
    }

    /// Return tracee by its number.
    ///
    /// # Arguments
    ///
    /// * `num`: tracee number
    pub fn get_tracee_by_num(&self, num: u32) -> Result<Tracee, Error> {
        let mut snapshot = self.tracee_ctl().snapshot();
        let tracee = snapshot.drain(..).find(|tracee| tracee.number == num);
        tracee.ok_or(TraceeNotFound(num))
    }

    /// Return debug information about program determined by program counter address.
    #[inline(always)]
    pub fn debug_info(&self, addr: RelocatedAddress) -> Result<&DebugInformation, Error> {
        self.dwarf_registry
            .find_by_addr(addr)
            .ok_or(Error::NoDebugInformation("current location"))
    }

    /// Return debug information about program determined by file which from it been parsed.
    #[inline(always)]
    pub fn debug_info_from_file(&self, path: &Path) -> Result<&DebugInformation, Error> {
        self.dwarf_registry
            .find_by_file(path)
            .ok_or(Error::NoDebugInformation("file"))
    }

    /// Get main executable object debug information.
    #[inline(always)]
    pub fn program_debug_info(&self) -> Result<&DebugInformation, Error> {
        self.dwarf_registry
            .find_main_program_dwarf()
            .ok_or(Error::NoDebugInformation("executable object"))
    }

    /// Return all known debug information.
    /// Debug info of the main executable is located at the zero index.
    /// Other information ordered from less compilation unit counts to greatest.
    #[inline(always)]
    pub fn debug_info_all(&self) -> Vec<&DebugInformation> {
        self.dwarf_registry.all_dwarf()
    }

    /// Return mapped memory region offset for region.
    ///
    /// # Arguments
    ///
    /// * `pc`: VAS address, determine region for which offset is needed.
    pub fn mapping_offset_for_pc(&self, addr: RelocatedAddress) -> Result<usize, Error> {
        self.dwarf_registry
            .find_mapping_offset(addr)
            .ok_or(MappingOffsetNotFound("address out of bounds"))
    }

    /// Return mapped memory region offset for region.
    ///
    /// # Arguments
    ///
    /// * `dwarf`: debug information (with file path inside) for determine memory region.
    pub fn mapping_offset_for_file(&self, dwarf: &DebugInformation) -> Result<usize, Error> {
        self.dwarf_registry
            .find_mapping_offset_for_file(dwarf)
            .ok_or(MappingOffsetNotFound("unknown segment"))
    }

    /// Unwind debugee thread stack and return a backtrace.
    ///
    /// # Arguments
    ///
    /// * `pid`: thread for unwinding
    pub fn unwind(&self, pid: Pid) -> Result<Backtrace, Error> {
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
    ) -> Result<(), Error> {
        unwind::restore_registers_at_frame(self, pid, registers, frame_num)
    }

    /// Return a current frame return address for current thread.
    ///
    /// # Arguments
    ///
    /// * `pid`: thread for unwinding
    #[allow(unused)]
    pub fn return_addr(&self, pid: Pid) -> Result<Option<RelocatedAddress>, Error> {
        unwind::return_addr(self, pid)
    }

    /// Return a ordered list of mapped regions (main executable region at first place).
    pub fn dump_mapped_regions(&self) -> Vec<RegionInfo> {
        self.dwarf_registry.dump()
    }

    /// Return a list of disassembled instruction for a function in focus.
    pub fn disasm(
        &self,
        ctx: &ExplorationContext,
        breakpoints: &[&Breakpoint],
    ) -> Result<FunctionAssembly, Error> {
        let debug_information = self.debug_info(ctx.location().pc)?;
        let function = debug_information
            .find_function_by_pc(ctx.location().global_pc)?
            .ok_or(FunctionNotFound(ctx.location().global_pc))?;

        let instructions =
            self.disassembly
                .disasm_function(self, debug_information, function, breakpoints)?;

        Ok(FunctionAssembly {
            name: function.full_name(),
            addr_in_focus: ctx.location().global_pc,
            instructions,
        })
    }

    /// Return two place descriptors, at the start and at the end of the current function.
    pub fn function_range(&self, ctx: &ExplorationContext) -> Result<FunctionRange, Error> {
        let debug_information = self.debug_info(ctx.location().pc)?;
        let function = debug_information
            .find_function_by_pc(ctx.location().global_pc)?
            .ok_or(FunctionNotFound(ctx.location().global_pc))?;
        let unit = function.unit();

        let stop_place = debug_information
            .find_place_from_pc(ctx.location().global_pc)?
            .ok_or(Error::PlaceNotFound(ctx.location().global_pc))?;

        let (file, start_line) = function.die.decl_file_line.ok_or(FunctionRangeNotFound)?;
        let file = unit.files()[file as usize].as_path();

        let fn_places: Vec<_> = function
            .die
            .base_attributes
            .ranges
            .iter()
            .flat_map(|range| function.unit().find_lines_for_range(range))
            .collect();

        let mut end_line = fn_places
            .iter()
            .map(|place| place.line_number)
            .max()
            .unwrap_or(start_line);

        if start_line > end_line {
            warn!(target: "debugger", "irrational function range ({start_line};{end_line})");
            end_line = start_line;
        }

        Ok(FunctionRange {
            name: function.full_name(),
            stop_place,
            file,
            start_line,
            end_line,
        })
    }
}

/// Parse dwarf information from new dependency.
fn parse_dependency(dep_file: impl Into<PathBuf>) -> Result<Option<DebugInformation>, Error> {
    let dep_file = dep_file.into();

    // empty string represents a program executable that must already parse
    // libvdso should also be skipped
    if dep_file.is_empty() || dep_file.to_string_lossy().contains("vdso") {
        return Ok(None);
    }

    let file = fs::File::open(&dep_file)?;
    let mmap = unsafe { memmap2::Mmap::map(&file)? };
    let object = object::File::parse(&*mmap)?;

    let dwarf_builder = dwarf::DebugInformationBuilder;
    let dwarf = dwarf_builder.build(dep_file.as_path(), &object)?;
    Ok(Some(dwarf))
}

/// Parse list of dependencies and add result into debug information registry.
fn parse_dependencies_into_registry(
    registry: &mut DwarfRegistry,
    deps: impl Iterator<Item = impl Into<PathBuf>>,
    quiet: bool,
) {
    let dwarfs: Vec<_> = deps
        .map(|dep| dep.into())
        .collect::<Vec<_>>()
        .into_par_iter()
        .filter_map(|dep| {
            let parse_result = parse_dependency(&dep);
            match parse_result {
                Ok(mb_dep) => mb_dep.map(|dwarf| {
                    if !quiet {
                        info!(target: "dwarf-loader", "load shared library {dep:?}");
                    }

                    (dep, dwarf)
                }),
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
