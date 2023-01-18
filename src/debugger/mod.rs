mod breakpoint;
mod code;
pub mod command;
mod dwarf;
mod register;
pub mod rust;
mod thread;
mod utils;
mod uw;
pub mod variable;

pub use dwarf::parser::unit::Place;
pub use dwarf::r#type::TypeDeclaration;

use crate::debugger::breakpoint::Breakpoint;
use crate::debugger::dwarf::parser::unit::{FunctionDie, VariableDie};
use crate::debugger::dwarf::r#type::{EvaluationContext, TypeDeclarationCache};
use crate::debugger::dwarf::{ContextualDieRef, DebugeeContext, EndianRcSlice, Symbol};
use crate::debugger::register::{
    get_register_from_name, get_register_value, set_register_value, Register,
};
use crate::debugger::thread::{Registry, TraceeStatus, TraceeThread};
use crate::debugger::uw::Backtrace;
use crate::debugger::variable::VariableIR;
use crate::weak_error;
use anyhow::anyhow;
use log::warn;
use nix::errno::Errno;
use nix::libc::{c_int, c_void, pid_t, uintptr_t};
use nix::sys::signal::Signal;
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::Pid;
use nix::{libc, sys};
use proc_maps::MapRange;
use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::ffi::c_long;
use std::path::{Path, PathBuf};
use std::{fs, mem, u64};

pub struct FrameInfo {
    pub base_addr: usize,
    pub return_addr: Option<usize>,
}

pub trait EventHook {
    fn on_trap(&self, pc: usize, place: Option<Place>) -> anyhow::Result<()>;
    fn on_signal(&self, signo: c_int, code: c_int);
    fn on_exit(&self, code: i32);
}

pub struct Debugger<T: EventHook> {
    hooks: T,
    debugee_path: PathBuf,
    debugee_mapping_addr: usize,
    breakpoints: HashMap<usize, Breakpoint>,
    dwarf: DebugeeContext<EndianRcSlice>,
    type_cache: RefCell<TypeDeclarationCache>,
    thread_registry: Registry,
}

impl<T: EventHook> Debugger<T> {
    pub fn new(program: impl Into<String>, pid: Pid, hooks: T) -> anyhow::Result<Self> {
        let program = program.into();
        let program_path = Path::new(&program);
        let file = fs::File::open(program_path)?;

        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        let object = object::File::parse(&*mmap)?;

        let dwarf_builder = dwarf::DebugeeContextBuilder::default();

        Ok(Self {
            debugee_path: program_path.to_path_buf(),
            debugee_mapping_addr: 0,
            breakpoints: HashMap::default(),
            dwarf: dwarf_builder.build(&object)?,
            hooks,
            type_cache: RefCell::default(),
            thread_registry: Registry::new(pid),
        })
    }

    fn init_mapping_addr(&mut self) -> anyhow::Result<()> {
        let absolute_debugee_path_buf = self.debugee_path.canonicalize()?;
        let absolute_debugee_path = absolute_debugee_path_buf.as_path();

        let proc_maps: Vec<MapRange> =
            proc_maps::get_process_maps(self.thread_registry.main_thread().as_raw())?
                .into_iter()
                .filter(|map| map.filename() == Some(absolute_debugee_path))
                .collect();

        let lowest_map = proc_maps
            .iter()
            .min_by(|map1, map2| map1.start().cmp(&map2.start()))
            .ok_or_else(|| anyhow!("mapping not found"))?;

        self.debugee_mapping_addr = lowest_map.start();
        Ok(())
    }

    fn get_symbol(&self, name: &str) -> anyhow::Result<&Symbol> {
        self.dwarf
            .find_symbol(name)
            .ok_or_else(|| anyhow!("symbol not found"))
    }

    fn frame_info(&self, pid: Pid) -> anyhow::Result<FrameInfo> {
        let func = self
            .dwarf
            .find_function_by_pc(self.offset_pc()?)
            .ok_or_else(|| anyhow!("current function not found"))?;

        Ok(FrameInfo {
            base_addr: func.frame_base_addr(pid)?,
            return_addr: uw::return_addr(pid)?,
        })
    }

    fn step_into(&self) -> anyhow::Result<()> {
        self.step_in()?;
        self.hooks.on_trap(
            self.offset_pc()?,
            self.dwarf.find_place_from_pc(self.offset_pc()?),
        )
    }

    fn stepi(&self) -> anyhow::Result<()> {
        self.single_step_instruction()?;
        self.hooks.on_trap(
            self.offset_pc()?,
            self.dwarf.find_place_from_pc(self.offset_pc()?),
        )
    }

    fn thread_state(&self) -> Vec<ThreadDump> {
        let threads = self.thread_registry.dump();
        threads
            .into_iter()
            .map(|thread| {
                let pc = weak_error!(get_register_value(thread.pid, Register::Rip));
                let bt = weak_error!(uw::backtrace(thread.pid));
                ThreadDump {
                    on_focus: thread.pid == self.thread_registry.on_focus_thread(),
                    thread,
                    pc,
                    bt,
                }
            })
            .collect()
    }

    fn backtrace(&self, pid: Pid) -> anyhow::Result<Backtrace> {
        Ok(uw::backtrace(pid)?)
    }

    fn offset_load_addr(&self, addr: usize) -> usize {
        addr - self.debugee_mapping_addr
    }

    fn offset_pc(&self) -> nix::Result<usize> {
        Ok(self.offset_load_addr(self.get_current_thread_pc()?))
    }

    fn continue_execution(&mut self) -> anyhow::Result<()> {
        self.step_over_breakpoint()?;
        self.thread_registry.cont_stopped()?;
        self.wait_for_signal()
    }

    fn set_breakpoint(&mut self, addr: usize) -> anyhow::Result<()> {
        let bp = Breakpoint::new(addr, self.thread_registry.main_thread());
        bp.enable()?;
        self.breakpoints.insert(addr, bp);
        Ok(())
    }

    fn remove_breakpoint(&mut self, addr: usize) -> anyhow::Result<()> {
        let bp = self.breakpoints.remove(&addr);
        if let Some(bp) = bp {
            if bp.is_enabled() {
                bp.disable()?;
            }
        }
        Ok(())
    }

    /// Read N bytes from debugee process.
    fn read_memory(&self, addr: usize, read_n: usize) -> nix::Result<Vec<u8>> {
        read_memory_by_pid(self.thread_registry.main_thread(), addr, read_n)
    }

    fn write_memory(&self, addr: uintptr_t, value: uintptr_t) -> nix::Result<()> {
        unsafe {
            sys::ptrace::write(
                self.thread_registry.main_thread(),
                addr as *mut c_void,
                value as *mut c_void,
            )
        }
    }

    fn get_current_thread_pc(&self) -> nix::Result<usize> {
        get_register_value(self.thread_registry.on_focus_thread(), Register::Rip)
            .map(|addr| addr as usize)
    }

    fn set_current_thread_pc(&self, value: usize) -> nix::Result<()> {
        set_register_value(
            self.thread_registry.on_focus_thread(),
            Register::Rip,
            value as u64,
        )
    }

    fn step_over_breakpoint(&self) -> anyhow::Result<()> {
        let current_pc = self.get_current_thread_pc()? as usize;
        let mb_bp = self.breakpoints.get(&current_pc);
        if let Some(bp) = mb_bp {
            if bp.is_enabled() {
                bp.disable()?;
                let on_focus_thread = self.thread_registry.on_focus_thread();
                sys::ptrace::step(on_focus_thread, None)?;
                let _status = waitpid(on_focus_thread, None)?;
                debug_assert!({
                    // assert TRAP_TRACE code
                    let info = sys::ptrace::getsiginfo(on_focus_thread);
                    matches!(WaitStatus::Stopped, _status)
                        && info
                            .map(|info| info.si_code == code::TRAP_TRACE)
                            .unwrap_or(false)
                });
                bp.enable()?;
            }
        }
        Ok(())
    }

    fn wait_for_signal(&mut self) -> anyhow::Result<()> {
        let status = waitpid(Pid::from_raw(-1), None)?;

        match status {
            WaitStatus::Exited(pid, code) => {
                // at this point thread must already removed from registry
                // anyway `registry.remove` is idempotent
                self.thread_registry.remove(pid);

                if pid == self.thread_registry.main_thread() {
                    self.hooks.on_exit(code);
                } else {
                    self.wait_for_signal()?;
                }
                Ok(())
            }
            WaitStatus::PtraceEvent(pid, _, code) => {
                match code {
                    libc::PTRACE_EVENT_EXEC => {
                        // fire just before debugee start
                        // cause currently `fork()` in debugee is unsupported we expect this code calling once
                        self.init_mapping_addr()?;
                        self.thread_registry.set_stop_status(pid);
                    }
                    libc::PTRACE_EVENT_CLONE => {
                        // fire just before new thread created
                        let tid = sys::ptrace::getevent(pid)?;
                        self.thread_registry.set_stop_status(pid);
                        self.thread_registry.register(Pid::from_raw(tid as pid_t));

                        self.wait_for_signal()?;
                    }
                    libc::PTRACE_EVENT_STOP => {
                        // fire right after new thread started or PTRACE_INTERRUPT called
                        if self.thread_registry.status(pid) == TraceeStatus::Created {
                            self.thread_registry.set_stop_status(pid);
                            self.thread_registry.cont_stopped()?;
                            self.wait_for_signal()?;
                        } else {
                            self.thread_registry.set_stop_status(pid);
                            self.wait_for_signal()?;
                        }
                    }
                    libc::PTRACE_EVENT_EXIT => {
                        // fire just before thread exit
                        self.thread_registry.set_stop_status(pid);
                        self.thread_registry.cont_stopped()?;
                        self.thread_registry.remove(pid);
                        self.wait_for_signal()?;
                    }
                    _ => {
                        warn!("unsupported ptrace event, code: {code}");
                        self.wait_for_signal()?;
                    }
                }

                Ok(())
            }

            WaitStatus::Stopped(pid, signal) => {
                let info = match sys::ptrace::getsiginfo(pid) {
                    Ok(info) => info,
                    Err(Errno::ESRCH) => return Ok(()),
                    Err(e) => return Err(e.into()),
                };

                match signal {
                    Signal::SIGTRAP => match info.si_code {
                        code::TRAP_TRACE => Ok(()),
                        code::TRAP_BRKPT | code::SI_KERNEL => {
                            self.thread_registry.set_in_focus_thread(pid);
                            self.thread_registry.set_stop_status(pid);
                            self.thread_registry.interrupt_running()?;

                            self.set_current_thread_pc(self.get_current_thread_pc()? - 1)?;
                            let current_pc = self.get_current_thread_pc()?;
                            let offset_pc = self.offset_load_addr(current_pc);
                            self.hooks
                                .on_trap(current_pc, self.dwarf.find_place_from_pc(offset_pc))
                        }
                        code => Err(anyhow!("unexpected SIGTRAP code {code}")),
                    },
                    _ => {
                        self.thread_registry.set_in_focus_thread(pid);
                        self.thread_registry.set_stop_status(pid);
                        self.thread_registry.interrupt_running()?;

                        self.hooks.on_signal(info.si_signo, info.si_code);
                        Ok(())
                    }
                }
            }
            _ => {
                warn!("unexpected wait status: {status:?}");
                self.wait_for_signal()
            }
        }
    }

    fn single_step_instruction(&self) -> anyhow::Result<()> {
        if self
            .breakpoints
            .get(&(self.get_current_thread_pc()? as usize))
            .is_some()
        {
            self.step_over_breakpoint()
        } else {
            sys::ptrace::step(self.thread_registry.on_focus_thread(), None)?;
            let _status = waitpid(self.thread_registry.on_focus_thread(), None)?;
            debug_assert!({
                // assert TRAP_TRACE code
                let info = sys::ptrace::getsiginfo(self.thread_registry.on_focus_thread());
                matches!(WaitStatus::Stopped, _status)
                    && info
                        .map(|info| info.si_code == code::TRAP_TRACE)
                        .unwrap_or(false)
            });
            Ok(())
        }
    }

    fn step_out(&mut self) -> anyhow::Result<()> {
        if let Some(ret_addr) = uw::return_addr(self.thread_registry.on_focus_thread())? {
            let bp_is_set = self.breakpoints.get(&(ret_addr as usize)).is_some();
            if bp_is_set {
                self.continue_execution()?;
            } else {
                self.set_breakpoint(ret_addr)?;
                self.continue_execution()?;
                self.remove_breakpoint(ret_addr)?;
            }
        }
        Ok(())
    }

    fn step_in(&self) -> anyhow::Result<()> {
        let place = self
            .dwarf
            .find_place_from_pc(self.offset_pc()?)
            .ok_or_else(|| anyhow!("not in debug frame (may be program not started?)"))?;

        while place
            == self
                .dwarf
                .find_place_from_pc(self.offset_pc()?)
                .ok_or_else(|| anyhow!("unreachable! line not found"))?
        {
            self.single_step_instruction()?
        }

        Ok(())
    }

    fn step_over(&mut self) -> anyhow::Result<()> {
        let func = self
            .dwarf
            .find_function_by_pc(self.offset_pc()?)
            .ok_or_else(|| anyhow!("not in debug frame (may be program not started?)"))?;

        let mut to_delete = vec![];

        let current_line = self
            .dwarf
            .find_place_from_pc(self.offset_pc()?)
            .ok_or_else(|| anyhow!("current line not found"))?;

        let mut breakpoints_range = vec![];

        for range in func.die.base_attributes.ranges.iter() {
            let mut line = self
                .dwarf
                .find_place_from_pc(range.begin as usize)
                .ok_or_else(|| anyhow!("unknown function range"))?;

            while line.address < range.end {
                if line.is_stmt {
                    let load_addr = self.offset_to_glob_addr(line.address as usize);
                    if line.address != current_line.address
                        && self.breakpoints.get(&load_addr).is_none()
                    {
                        breakpoints_range.push(load_addr);
                        to_delete.push(load_addr);
                    }
                }

                match line.next() {
                    None => break,
                    Some(n) => line = n,
                }
            }
        }

        breakpoints_range
            .into_iter()
            .try_for_each(|load_addr| self.set_breakpoint(load_addr))?;

        if let Some(ret_addr) = uw::return_addr(self.thread_registry.on_focus_thread())? {
            if self.breakpoints.get(&ret_addr).is_none() {
                self.set_breakpoint(ret_addr)?;
                to_delete.push(ret_addr);
            }
        }

        self.continue_execution()?;

        to_delete
            .into_iter()
            .try_for_each(|addr| self.remove_breakpoint(addr))?;

        Ok(())
    }

    fn set_breakpoint_at_fn(&mut self, name: &str) -> anyhow::Result<()> {
        let func = self
            .dwarf
            .find_function_by_name(name)
            .ok_or_else(|| anyhow!("function not found"))?;

        // todo find range with lowes begin
        let low_pc = func.die.base_attributes.ranges[0].begin;
        let entry = self
            .dwarf
            .find_place_from_pc(low_pc as usize)
            .ok_or_else(|| anyhow!("invalid function entry"))?
            // TODO skip prologue smarter
            .next()
            .ok_or_else(|| anyhow!("invalid function entry"))?;

        self.set_breakpoint(self.offset_to_glob_addr(entry.address as usize))
    }

    fn set_breakpoint_at_line(&mut self, fine_name: &str, line: u64) -> anyhow::Result<()> {
        if let Some(place) = self.dwarf.find_stmt_line(fine_name, line) {
            self.set_breakpoint(self.offset_to_glob_addr(place.address as usize))?;
        }
        Ok(())
    }

    fn offset_to_glob_addr(&self, addr: usize) -> usize {
        addr + self.debugee_mapping_addr
    }

    // Read all local variables from current thread.
    fn read_local_variables(&self) -> anyhow::Result<Vec<VariableIR>> {
        let pc = self.offset_pc()?;
        let current_func = self
            .dwarf
            .find_function_by_pc(pc)
            .ok_or_else(|| anyhow!("not in function"))?;
        let vars = current_func.find_variables(pc);
        self.variables_into_variable_ir(&vars, Some(current_func))
    }

    // Read any variable from current thread.
    fn read_variable(&self, name: &str) -> anyhow::Result<Vec<VariableIR>> {
        let vars = self.dwarf.find_variables(name);
        self.variables_into_variable_ir(&vars, None)
    }

    fn variables_into_variable_ir(
        &self,
        vars: &[ContextualDieRef<VariableDie>],
        known_parent_fn: Option<ContextualDieRef<FunctionDie>>,
    ) -> anyhow::Result<Vec<VariableIR>> {
        let mut type_cache = self.type_cache.borrow_mut();

        let vars = vars
            .iter()
            .map(|var| {
                let mb_type = var.die.type_ref.and_then(|type_ref| {
                    match type_cache.entry((var.unit.id, type_ref)) {
                        Entry::Occupied(o) => Some(&*o.into_mut()),
                        Entry::Vacant(v) => var.r#type().map(|t| &*v.insert(t)),
                    }
                });

                let mb_value = mb_type.as_ref().and_then(|type_decl| {
                    var.read_value_at_location(
                        self.thread_registry.on_focus_thread(),
                        type_decl,
                        known_parent_fn.or_else(|| var.assume_parent_function()),
                        self.debugee_mapping_addr,
                    )
                });

                VariableIR::new(
                    &EvaluationContext {
                        unit: var.unit,
                        pid: self.thread_registry.on_focus_thread(),
                    },
                    var.die.base_attributes.name.clone(),
                    mb_value,
                    mb_type,
                )
            })
            .collect();
        Ok(vars)
    }

    pub fn get_register_value(&self, register_name: &str) -> anyhow::Result<u64> {
        Ok(get_register_value(
            self.thread_registry.on_focus_thread(),
            get_register_from_name(register_name)?,
        )?)
    }

    pub fn set_register_value(&self, register_name: &str, val: u64) -> anyhow::Result<()> {
        Ok(set_register_value(
            self.thread_registry.on_focus_thread(),
            get_register_from_name(register_name)?,
            val,
        )?)
    }
}

/// Read N bytes from `PID` process.
pub fn read_memory_by_pid(pid: Pid, addr: usize, read_n: usize) -> nix::Result<Vec<u8>> {
    let mut read_reminder = read_n as isize;
    let mut result = Vec::with_capacity(read_n);

    let single_read_size = mem::size_of::<c_long>();

    let mut addr = addr as *mut c_long;
    while read_reminder > 0 {
        let value = sys::ptrace::read(pid, addr as *mut c_void)?;
        result.extend(value.to_ne_bytes().into_iter().take(read_reminder as usize));

        read_reminder -= single_read_size as isize;
        addr = unsafe { addr.offset(1) };
    }

    debug_assert!(result.len() == read_n);

    Ok(result)
}

pub struct ThreadDump {
    pub thread: TraceeThread,
    pub pc: Option<u64>,
    pub bt: Option<Backtrace>,
    pub on_focus: bool,
}
