mod breakpoint;
pub mod command;
mod dwarf;
mod register;
pub mod ui;
mod utils;
mod uw;

use crate::debugger::breakpoint::Breakpoint;
use crate::debugger::dwarf::parse::Place;
use crate::debugger::dwarf::r#type::TypeDeclaration;
use crate::debugger::dwarf::{DebugeeContext, EndianRcSlice, Symbol};
use crate::debugger::register::{
    get_register_from_name, get_register_value, set_register_value, Register,
};
use crate::debugger::uw::Backtrace;
use anyhow::anyhow;
use bytes::Bytes;
use nix::errno::Errno;
use nix::libc::{c_void, siginfo_t, uintptr_t};
use nix::sys::wait::waitpid;
use nix::unistd::Pid;
use nix::{libc, sys};
use object::{Object, ObjectKind};
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::str::from_utf8;
use std::{fs, u64};

pub struct FrameInfo {
    pub base_addr: usize,
    pub return_addr: Option<usize>,
}

pub trait EventHook {
    fn on_sigtrap(&self, pc: usize, place: Option<Place>) -> anyhow::Result<()>;
}

pub struct Debugger<'a, T: EventHook> {
    _program: &'a str,
    load_addr: Cell<usize>,
    pid: Pid,
    breakpoints: RefCell<HashMap<usize, Breakpoint>>,
    obj_kind: object::ObjectKind,
    dwarf: DebugeeContext<EndianRcSlice>,
    event_hooks: T,
}

impl<'a, T: EventHook> Debugger<'a, T> {
    pub fn new(program: &'a str, pid: Pid, hooks: T) -> Self {
        let file = fs::File::open(program).unwrap();
        let mmap = unsafe { memmap2::Mmap::map(&file).unwrap() };
        let object = object::File::parse(&*mmap).unwrap();

        Self {
            load_addr: Cell::new(0),
            _program: program,
            pid,
            breakpoints: Default::default(),
            dwarf: DebugeeContext::new(&object).unwrap(),
            obj_kind: object.kind(),
            event_hooks: hooks,
        }
    }

    fn init_load_addr(&self) -> anyhow::Result<()> {
        if self.obj_kind == ObjectKind::Dynamic {
            let addrs = fs::read(format!("/proc/{}/maps", self.pid))?;
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

    pub fn on_debugee_start(&self) -> anyhow::Result<()> {
        waitpid(self.pid, None)?;
        self.init_load_addr()
    }

    fn get_symbol(&self, name: &str) -> anyhow::Result<&Symbol> {
        self.dwarf
            .find_symbol(name)
            .ok_or_else(|| anyhow!("symbol not found"))
    }

    fn frame_info(&self) -> anyhow::Result<FrameInfo> {
        let func = self
            .dwarf
            .find_function_by_pc(self.offset_pc()?)
            .ok_or_else(|| anyhow!("current function not found"))?;

        Ok(FrameInfo {
            base_addr: func.frame_base_addr(self.pid)?,
            return_addr: uw::return_addr(self.pid)?,
        })
    }

    fn step_into(&self) -> anyhow::Result<Option<Place>> {
        self.step_in()?;
        Ok(self.dwarf.find_place_from_pc(self.offset_pc()?))
    }

    fn stepi(&self) -> anyhow::Result<Option<Place>> {
        self.single_step_instruction()?;
        let offset_pc = self.offset_pc()?;
        Ok(self.dwarf.find_place_from_pc(offset_pc))
    }

    fn backtrace(&self) -> anyhow::Result<Backtrace> {
        Ok(uw::backtrace(self.pid)?)
    }

    fn offset_load_addr(&self, addr: usize) -> usize {
        addr - self.load_addr.get()
    }

    fn offset_pc(&self) -> nix::Result<usize> {
        Ok(self.offset_load_addr(self.get_pc()?))
    }

    pub fn continue_execution(&self) -> anyhow::Result<()> {
        self.step_over_breakpoint()?;
        sys::ptrace::cont(self.pid, None)?;
        self.wait_for_signal()
    }

    fn set_breakpoint(&self, addr: usize) -> anyhow::Result<()> {
        let bp = Breakpoint::new(addr, self.pid);
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

    fn read_memory(&self, addr: usize) -> nix::Result<uintptr_t> {
        sys::ptrace::read(self.pid, addr as *mut c_void).map(|v| v as uintptr_t)
    }

    fn write_memory(&self, addr: uintptr_t, value: uintptr_t) -> nix::Result<()> {
        unsafe { sys::ptrace::write(self.pid, addr as *mut c_void, value as *mut c_void) }
    }

    fn get_pc(&self) -> nix::Result<usize> {
        get_register_value(self.pid, Register::Rip).map(|addr| addr as usize)
    }

    fn set_pc(&self, value: usize) -> nix::Result<()> {
        set_register_value(self.pid, Register::Rip, value as u64)
    }

    fn step_over_breakpoint(&self) -> anyhow::Result<()> {
        let breakpoints = self.breakpoints.borrow();
        let mb_bp = breakpoints.get(&(self.get_pc()? as usize));
        if let Some(bp) = mb_bp {
            if bp.is_enabled() {
                bp.disable()?;
                sys::ptrace::step(self.pid, None)?;
                self.wait_for_signal()?;
                bp.enable()?;
            }
        }
        Ok(())
    }

    fn wait_for_signal(&self) -> anyhow::Result<()> {
        waitpid(self.pid, None)?;

        let info = match sys::ptrace::getsiginfo(self.pid) {
            Ok(info) => info,
            Err(Errno::ESRCH) => return Ok(()),
            Err(e) => return Err(e.into()),
        };

        match info.si_signo {
            libc::SIGTRAP => self.handle_sigtrap(info)?,
            libc::SIGSEGV => println!("Segfault! Reason: {}", info.si_code),
            _ => println!("Receive signal: {}", info.si_signo),
        }
        Ok(())
    }

    fn handle_sigtrap(&self, info: siginfo_t) -> anyhow::Result<()> {
        match info.si_code {
            0x80 | 0x1 => {
                self.set_pc(self.get_pc()? - 1)?;
                let current_pc = self.get_pc()?;
                let offset_pc = self.offset_load_addr(current_pc);
                self.event_hooks
                    .on_sigtrap(current_pc, self.dwarf.find_place_from_pc(offset_pc))?;

                Ok(())
            }
            0x2 => Ok(()),
            _ => Err(anyhow!("Unknown SIGTRAP code: {}", info.si_code)),
        }
    }

    fn single_step_instruction(&self) -> anyhow::Result<()> {
        if self
            .breakpoints
            .borrow()
            .get(&(self.get_pc()? as usize))
            .is_some()
        {
            self.step_over_breakpoint()
        } else {
            sys::ptrace::step(self.pid, None)?;
            self.wait_for_signal()
        }
    }

    fn step_out(&self) -> anyhow::Result<()> {
        if let Some(ret_addr) = uw::return_addr(self.pid)? {
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

        //todo may be function have not single range, rewrite with respect of multiple ranges

        let mut line = self
            .dwarf
            .find_place_from_pc(func.die.base_attributes.ranges[0].begin as usize)
            .unwrap();
        let current_line = self.dwarf.find_place_from_pc(self.offset_pc()?).unwrap();

        let mut to_delete = vec![];
        while line.address < func.die.base_attributes.ranges[0].end {
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

        if let Some(ret_addr) = uw::return_addr(self.pid)? {
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

    fn read_variables(&self) -> anyhow::Result<Vec<Variable>> {
        let pc = self.offset_pc()?;

        let current_func = self
            .dwarf
            .find_function_by_pc(pc)
            .ok_or_else(|| anyhow!("not in function"))?;

        current_func
            .find_variables(pc)
            .iter()
            .map(|var| {
                let mb_type = var.r#type();
                let value = var.read_value_at_location(current_func, self.pid);
                Ok(Variable {
                    r#type: mb_type,
                    name: var.die.base_attributes.name.clone().map(Cow::Owned),
                    value,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()
    }

    pub fn get_register_value(&self, register_name: &str) -> anyhow::Result<u64> {
        Ok(get_register_value(
            self.pid,
            get_register_from_name(register_name)?,
        )?)
    }

    pub fn set_register_value(&self, register_name: &str, val: u64) -> anyhow::Result<()> {
        Ok(set_register_value(
            self.pid,
            get_register_from_name(register_name)?,
            val,
        )?)
    }
}

pub struct Variable<'a> {
    name: Option<Cow<'a, str>>,
    r#type: Option<TypeDeclaration>,
    value: Option<Bytes>,
}
