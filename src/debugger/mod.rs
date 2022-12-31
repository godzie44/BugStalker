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
use crate::debugger::dwarf::r#type::{EvaluationContext, TypeDeclarationCache};
use crate::debugger::dwarf::{DebugeeContext, EndianRcSlice, Symbol};
use crate::debugger::register::{
    get_register_from_name, get_register_value, set_register_value, Register,
};
use crate::debugger::thread::{Registry, TraceeStatus};
use crate::debugger::uw::Backtrace;
use crate::debugger::variable::VariableIR;
use anyhow::anyhow;
use log::warn;
use nix::errno::Errno;
use nix::libc::{c_int, c_void, pid_t, uintptr_t};
use nix::sys::signal::Signal;
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::Pid;
use nix::{libc, sys};
use object::{Object, ObjectKind};
use std::cell::{Cell, RefCell};
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::ffi::c_long;
use std::str::from_utf8;
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
    _program: String,
    load_addr: Cell<usize>,
    breakpoints: RefCell<HashMap<usize, Breakpoint>>,
    obj_kind: object::ObjectKind,
    dwarf: DebugeeContext<EndianRcSlice>,
    hooks: T,
    type_cache: RefCell<TypeDeclarationCache>,

    thread_registry: Registry,
}

impl<T: EventHook> Debugger<T> {
    pub fn new(program: impl Into<String>, pid: Pid, hooks: T) -> anyhow::Result<Self> {
        let program = program.into();
        let file = fs::File::open(&program)?;
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        let object = object::File::parse(&*mmap)?;

        let dwarf_builder = dwarf::DebugeeContextBuilder::default();

        Ok(Self {
            load_addr: Cell::new(0),
            _program: program,
            breakpoints: Default::default(),
            dwarf: dwarf_builder.build(&object)?,
            obj_kind: object.kind(),
            hooks,
            type_cache: RefCell::default(),
            thread_registry: Registry::new(pid),
        })
    }

    fn init_load_addr(&self) -> anyhow::Result<()> {
        if self.obj_kind == ObjectKind::Dynamic {
            let addrs = fs::read(format!("/proc/{}/maps", self.thread_registry.main_thread()))?;
            let maps = from_utf8(&addrs)?;
            let first_line = maps
                .lines()
                .next()
                .ok_or_else(|| anyhow!("unexpected line format"))?;
            let addr = first_line
                .split('-')
                .next()
                .ok_or_else(|| anyhow!("unexpected line format"))?;
            let addr = usize::from_str_radix(addr, 16)?;
            self.load_addr.set(addr);
        }
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

    fn backtrace(&self, pid: Pid) -> anyhow::Result<Backtrace> {
        Ok(uw::backtrace(pid)?)
    }

    fn offset_load_addr(&self, addr: usize) -> usize {
        addr - self.load_addr.get()
    }

    fn offset_pc(&self) -> nix::Result<usize> {
        Ok(self.offset_load_addr(self.get_current_thread_pc()?))
    }

    fn continue_execution(&self) -> anyhow::Result<()> {
        self.step_over_breakpoint()?;
        self.thread_registry.cont_stopped()?;
        self.wait_for_signal()
    }

    fn set_breakpoint(&self, addr: usize) -> anyhow::Result<()> {
        let bp = Breakpoint::new(addr, self.thread_registry.main_thread());
        bp.enable()?;
        self.breakpoints.borrow_mut().insert(addr, bp);
        Ok(())
    }

    fn remove_breakpoint(&self, addr: usize) -> anyhow::Result<()> {
        let bp = self.breakpoints.borrow_mut().remove(&addr);
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
        let breakpoints = self.breakpoints.borrow();
        let current_pc = self.get_current_thread_pc()? as usize;
        let mb_bp = breakpoints.get(&current_pc);
        if let Some(bp) = mb_bp {
            if bp.is_enabled() {
                bp.disable()?;
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

                bp.enable()?;
            }
        }
        Ok(())
    }

    fn wait_for_signal(&self) -> anyhow::Result<()> {
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
                        self.init_load_addr()?;
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
            .borrow()
            .get(&(self.get_current_thread_pc()? as usize))
            .is_some()
        {
            self.step_over_breakpoint()
        } else {
            sys::ptrace::step(self.thread_registry.on_focus_thread(), None)?;
            self.wait_for_signal()
        }
    }

    fn step_out(&self) -> anyhow::Result<()> {
        if let Some(ret_addr) = uw::return_addr(self.thread_registry.on_focus_thread())? {
            let bp_is_set = self
                .breakpoints
                .borrow()
                .get(&(ret_addr as usize))
                .is_some();
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

    fn step_over(&self) -> anyhow::Result<()> {
        let func = self
            .dwarf
            .find_function_by_pc(self.offset_pc()?)
            .ok_or_else(|| anyhow!("not in debug frame (may be program not started?)"))?;

        let mut to_delete = vec![];

        let current_line = self
            .dwarf
            .find_place_from_pc(self.offset_pc()?)
            .ok_or_else(|| anyhow!("current line not found"))?;

        for range in func.die.base_attributes.ranges.iter() {
            let mut line = self
                .dwarf
                .find_place_from_pc(range.begin as usize)
                .ok_or_else(|| anyhow!("unknown function range"))?;

            while line.address < range.end {
                if line.is_stmt {
                    let load_addr = self.offset_to_glob_addr(line.address as usize);
                    if line.address != current_line.address
                        && self.breakpoints.borrow().get(&load_addr).is_none()
                    {
                        self.set_breakpoint(load_addr)?;
                        to_delete.push(load_addr);
                    }
                }

                match line.next() {
                    None => break,
                    Some(n) => line = n,
                }
            }
        }

        if let Some(ret_addr) = uw::return_addr(self.thread_registry.on_focus_thread())? {
            if self.breakpoints.borrow().get(&ret_addr).is_none() {
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

    fn set_breakpoint_at_fn(&self, name: &str) -> anyhow::Result<()> {
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

    fn set_breakpoint_at_line(&self, fine_name: &str, line: u64) -> anyhow::Result<()> {
        if let Some(place) = self.dwarf.find_stmt_line(fine_name, line) {
            self.set_breakpoint(self.offset_to_glob_addr(place.address as usize))?;
        }
        Ok(())
    }

    fn offset_to_glob_addr(&self, addr: usize) -> usize {
        addr + self.load_addr.get()
    }

    // Read all actual variables in current thread.
    fn read_variables(&self) -> anyhow::Result<Vec<VariableIR>> {
        let pc = self.offset_pc()?;

        let current_func = self
            .dwarf
            .find_function_by_pc(pc)
            .ok_or_else(|| anyhow!("not in function"))?;

        let vars = current_func.find_variables(pc);
        let mut type_cache = self.type_cache.borrow_mut();

        let vars = vars
            .into_iter()
            .map(|var| {
                let mb_type = var.die.type_ref.and_then(|type_ref| {
                    match type_cache.entry((var.unit.id, type_ref)) {
                        Entry::Occupied(o) => Some(&*o.into_mut()),
                        Entry::Vacant(v) => var.r#type().map(|t| &*v.insert(t)),
                    }
                });

                let mb_value = mb_type.as_ref().and_then(|type_decl| {
                    var.read_value_at_location(
                        type_decl,
                        current_func,
                        self.thread_registry.on_focus_thread(),
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
            .collect::<Vec<_>>();
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
