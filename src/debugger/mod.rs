mod breakpoint;
mod code;
pub mod command;
mod debugee;
mod debugee_ctl;
mod register;
pub mod rust;
mod utils;
mod uw;
pub mod variable;

pub use debugee::dwarf::parser::unit::Place;
pub use debugee::dwarf::r#type::TypeDeclaration;

use crate::debugger::breakpoint::Breakpoint;
use crate::debugger::debugee::dwarf::parser::unit::{FunctionDie, VariableDie};
use crate::debugger::debugee::dwarf::r#type::{EvaluationContext, TypeDeclarationCache};
use crate::debugger::debugee::dwarf::{ContextualDieRef, Symbol};
use crate::debugger::debugee::thread::TraceeThread;
use crate::debugger::debugee::Debugee;
use crate::debugger::debugee_ctl::{DebugeeControlFlow, DebugeeState};
use crate::debugger::register::{
    get_register_from_name, get_register_value, set_register_value, Register,
};
use crate::debugger::uw::Backtrace;
use crate::debugger::variable::VariableIR;
use crate::weak_error;
use anyhow::anyhow;
use nix::libc::{c_int, c_void, uintptr_t};
use nix::sys;
use nix::unistd::Pid;
use object::Object;
use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::ffi::c_long;
use std::path::Path;
use std::{fs, mem, u64};

pub struct FrameInfo {
    pub base_addr: usize,
    pub return_addr: Option<RelocatedAddress>,
}

pub struct ThreadDump {
    pub thread: TraceeThread,
    pub pc: Option<RelocatedAddress>,
    pub bt: Option<Backtrace>,
    pub in_focus: bool,
}

pub trait EventHook {
    fn on_trap(&self, pc: RelocatedAddress, place: Option<Place>) -> anyhow::Result<()>;
    fn on_signal(&self, signo: c_int, code: c_int);
    fn on_exit(&self, code: i32);
}

macro_rules! disable_when_not_stared {
    ($this: expr) => {
        use anyhow::bail;
        if !$this.debugee.in_progress {
            bail!("The program is not being started.")
        }
    };
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub struct RelocatedAddress(pub usize);

impl RelocatedAddress {
    pub fn to_global(self, offset: usize) -> GlobalAddress {
        GlobalAddress(self.0 - offset)
    }
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub struct GlobalAddress(pub usize);

impl GlobalAddress {
    pub fn relocate(self, offset: usize) -> RelocatedAddress {
        RelocatedAddress(self.0 + offset)
    }
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub enum PCValue {
    Relocated(RelocatedAddress),
    Global(GlobalAddress),
}

/// Main structure of bug-stalker, control debugee state and provides application functionality.
pub struct Debugger {
    /// Debugee static and runtime state.
    debugee: Debugee,
    /// Debugee control flow.
    debugee_flow: DebugeeControlFlow,
    /// Active and non-active breakpoint list.
    breakpoints: HashMap<PCValue, Breakpoint>,
    /// Type declaration cache.
    type_cache: RefCell<TypeDeclarationCache>,
    /// Debugger interrupt with UI by EventHook trait.
    hooks: Box<dyn EventHook>,
}

impl Debugger {
    pub fn new(
        program: impl Into<String>,
        pid: Pid,
        hooks: impl EventHook + 'static,
    ) -> anyhow::Result<Self> {
        let program = program.into();
        let program_path = Path::new(&program);

        let file = fs::File::open(program_path)?;
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        let object = object::File::parse(&*mmap)?;

        let entry_point = GlobalAddress(object.entry() as usize);
        let breakpoints = HashMap::from([(
            PCValue::Global(entry_point),
            breakpoint::Breakpoint::new(PCValue::Global(entry_point), pid),
        )]);

        Ok(Self {
            breakpoints,
            hooks: Box::new(hooks),
            type_cache: RefCell::default(),
            debugee_flow: DebugeeControlFlow::new(pid, entry_point),
            debugee: Debugee::new_non_running(program_path, pid, &object)?,
        })
    }

    fn continue_execution(&mut self) -> anyhow::Result<()> {
        self.step_over_breakpoint()?;
        self.debugee.threads_ctl.cont_stopped()?;

        loop {
            let debugee_state = self.debugee_flow.tick(self)?;
            self.debugee.apply_state(debugee_state)?;
            match debugee_state {
                DebugeeState::ThreadExit(_)
                | DebugeeState::BeforeNewThread(_, _)
                | DebugeeState::ThreadInterrupt(_)
                | DebugeeState::BeforeThreadExit(_) => {}
                DebugeeState::DebugeeStart => {
                    let mut brkpts_to_reloc = HashMap::with_capacity(self.breakpoints.len());
                    let keys = self.breakpoints.keys().copied().collect::<Vec<_>>();
                    for k in keys {
                        if let PCValue::Global(addr) = k {
                            brkpts_to_reloc.insert(addr, self.breakpoints.remove(&k).unwrap());
                        }
                    }
                    for (addr, mut brkpt) in brkpts_to_reloc {
                        brkpt.addr =
                            PCValue::Relocated(addr.relocate(self.debugee.mapping_offset()));
                        self.breakpoints.insert(brkpt.addr, brkpt);
                    }
                    self.breakpoints
                        .iter()
                        .try_for_each(|(_, brkpt)| brkpt.enable())?;

                    debug_assert!(self
                        .breakpoints
                        .iter()
                        .all(|(addr, _)| matches!(addr, PCValue::Relocated(_))));

                    self.debugee.threads_ctl.cont_stopped()?;
                }
                DebugeeState::UnexpectedPtraceEvent | DebugeeState::UnexpectedWaitStatus => {
                    self.debugee.threads_ctl.cont_stopped()?;
                }
                DebugeeState::NoSuchProcess | DebugeeState::TrapTrace => {
                    break;
                }
                DebugeeState::DebugeeExit(code) => {
                    self.hooks.on_exit(code);
                    break;
                }
                DebugeeState::Breakpoint(_) => {
                    //  self.set_current_thread_pc(self.get_current_thread_pc()?.0 - 1)?;
                    let current_pc = self.get_current_thread_pc()?;
                    let offset_pc = current_pc.to_global(self.debugee.mapping_offset());
                    self.hooks
                        .on_trap(current_pc, self.debugee.dwarf.find_place_from_pc(offset_pc))?;
                    break;
                }
                DebugeeState::AtEntryPoint(_) => {
                    self.step_over_breakpoint()?;
                    self.debugee.threads_ctl.cont_stopped()?;
                }
                DebugeeState::OsSignal(info, _) => {
                    self.hooks.on_signal(info.si_signo, info.si_code);
                    break;
                }
            }
        }

        Ok(())
    }

    pub fn run_debugee(&mut self) -> anyhow::Result<()> {
        self.continue_execution()
    }

    pub fn continue_debugee(&mut self) -> anyhow::Result<()> {
        disable_when_not_stared!(self);
        self.continue_execution()
    }

    pub fn get_symbol(&self, name: &str) -> anyhow::Result<&Symbol> {
        self.debugee
            .dwarf
            .find_symbol(name)
            .ok_or_else(|| anyhow!("symbol not found"))
    }

    pub fn frame_info(&self, pid: Pid) -> anyhow::Result<FrameInfo> {
        disable_when_not_stared!(self);
        let func = self
            .debugee
            .dwarf
            .find_function_by_pc(
                self.get_current_thread_pc()?
                    .to_global(self.debugee.mapping_offset()),
            )
            .ok_or_else(|| anyhow!("current function not found"))?;

        Ok(FrameInfo {
            base_addr: func.frame_base_addr(pid)?,
            return_addr: uw::return_addr(pid)?,
        })
    }

    pub fn step_into(&self) -> anyhow::Result<()> {
        disable_when_not_stared!(self);
        self.step_in()?;
        self.hooks.on_trap(
            self.get_current_thread_pc()?,
            self.debugee.dwarf.find_place_from_pc(
                self.get_current_thread_pc()?
                    .to_global(self.debugee.mapping_offset()),
            ),
        )
    }

    pub fn stepi(&self) -> anyhow::Result<()> {
        disable_when_not_stared!(self);
        self.single_step_instruction()?;
        self.hooks.on_trap(
            self.get_current_thread_pc()?,
            self.debugee.dwarf.find_place_from_pc(
                self.get_current_thread_pc()?
                    .to_global(self.debugee.mapping_offset()),
            ),
        )
    }

    pub fn thread_state(&self) -> anyhow::Result<Vec<ThreadDump>> {
        disable_when_not_stared!(self);
        let threads = self.debugee.threads_ctl.dump();
        Ok(threads
            .into_iter()
            .map(|thread| {
                let pc = weak_error!(get_register_value(thread.pid, Register::Rip));
                let bt = weak_error!(uw::backtrace(thread.pid));
                ThreadDump {
                    in_focus: thread.pid == self.debugee.threads_ctl.thread_in_focus(),
                    thread,
                    pc: pc.map(|pc| RelocatedAddress(pc as usize)),
                    bt,
                }
            })
            .collect())
    }

    pub fn backtrace(&self, pid: Pid) -> anyhow::Result<Backtrace> {
        disable_when_not_stared!(self);
        Ok(uw::backtrace(pid)?)
    }

    pub fn set_breakpoint(&mut self, addr: PCValue) -> anyhow::Result<()> {
        let brkpt = Breakpoint::new(addr, self.debugee.threads_ctl.proc_pid());
        if self.debugee.in_progress {
            brkpt.enable()?;
        }
        self.breakpoints.insert(addr, brkpt);
        Ok(())
    }

    pub fn remove_breakpoint(&mut self, addr: PCValue) -> anyhow::Result<()> {
        let brkpt = self.breakpoints.remove(&addr);
        if let Some(brkpt) = brkpt {
            if brkpt.is_enabled() {
                brkpt.disable()?;
            }
        }
        Ok(())
    }

    /// Read N bytes from debugee process.
    pub fn read_memory(&self, addr: usize, read_n: usize) -> anyhow::Result<Vec<u8>> {
        disable_when_not_stared!(self);
        Ok(read_memory_by_pid(
            self.debugee.threads_ctl.proc_pid(),
            addr,
            read_n,
        )?)
    }

    pub fn write_memory(&self, addr: uintptr_t, value: uintptr_t) -> anyhow::Result<()> {
        disable_when_not_stared!(self);
        unsafe {
            Ok(sys::ptrace::write(
                self.debugee.threads_ctl.proc_pid(),
                addr as *mut c_void,
                value as *mut c_void,
            )?)
        }
    }

    fn get_current_thread_pc(&self) -> nix::Result<RelocatedAddress> {
        self.get_thread_pc(self.debugee.threads_ctl.thread_in_focus())
    }

    fn get_thread_pc(&self, tid: Pid) -> nix::Result<RelocatedAddress> {
        get_register_value(tid, Register::Rip).map(|addr| RelocatedAddress(addr as usize))
    }

    #[allow(unused)]
    fn set_current_thread_pc(&self, value: usize) -> nix::Result<()> {
        self.set_thread_pc(self.debugee.threads_ctl.thread_in_focus(), value)
    }

    fn set_thread_pc(&self, tid: Pid, value: usize) -> nix::Result<()> {
        set_register_value(tid, Register::Rip, value as u64)
    }

    fn step_over_breakpoint(&self) -> anyhow::Result<()> {
        let current_pc = self.get_current_thread_pc()?;
        let mb_brkpt = self.breakpoints.get(&PCValue::Relocated(current_pc));
        if let Some(brkpt) = mb_brkpt {
            if brkpt.is_enabled() {
                brkpt.disable()?;
                DebugeeControlFlow::thread_step(self.debugee.threads_ctl.thread_in_focus())?;
                brkpt.enable()?;
            }
        }
        Ok(())
    }

    fn single_step_instruction(&self) -> anyhow::Result<()> {
        if self
            .breakpoints
            .get(&PCValue::Relocated(self.get_current_thread_pc()?))
            .is_some()
        {
            self.step_over_breakpoint()
        } else {
            DebugeeControlFlow::thread_step(self.debugee.threads_ctl.thread_in_focus())?;
            Ok(())
        }
    }

    pub fn step_out(&mut self) -> anyhow::Result<()> {
        disable_when_not_stared!(self);
        if let Some(ret_addr) = uw::return_addr(self.debugee.threads_ctl.thread_in_focus())? {
            let brkpt_is_set = self
                .breakpoints
                .get(&PCValue::Relocated(ret_addr))
                .is_some();
            if brkpt_is_set {
                self.continue_execution()?;
            } else {
                self.set_breakpoint(PCValue::Relocated(ret_addr))?;
                self.continue_execution()?;
                self.remove_breakpoint(PCValue::Relocated(ret_addr))?;
            }
        }
        Ok(())
    }

    pub fn step_in(&self) -> anyhow::Result<()> {
        disable_when_not_stared!(self);
        let place = self
            .debugee
            .dwarf
            .find_place_from_pc(
                self.get_current_thread_pc()?
                    .to_global(self.debugee.mapping_offset()),
            )
            .ok_or_else(|| anyhow!("not in debug frame (may be program not started?)"))?;

        while place
            == self
                .debugee
                .dwarf
                .find_place_from_pc(
                    self.get_current_thread_pc()?
                        .to_global(self.debugee.mapping_offset()),
                )
                .ok_or_else(|| anyhow!("unreachable! line not found"))?
        {
            self.single_step_instruction()?
        }

        Ok(())
    }

    pub fn step_over(&mut self) -> anyhow::Result<()> {
        disable_when_not_stared!(self);
        let func = self
            .debugee
            .dwarf
            .find_function_by_pc(
                self.get_current_thread_pc()?
                    .to_global(self.debugee.mapping_offset()),
            )
            .ok_or_else(|| anyhow!("not in debug frame (may be program not started?)"))?;

        let mut to_delete = vec![];

        let current_line = self
            .debugee
            .dwarf
            .find_place_from_pc(
                self.get_current_thread_pc()?
                    .to_global(self.debugee.mapping_offset()),
            )
            .ok_or_else(|| anyhow!("current line not found"))?;

        let mut breakpoints_range = vec![];

        for range in func.die.base_attributes.ranges.iter() {
            let mut line = self
                .debugee
                .dwarf
                .find_place_from_pc(GlobalAddress(range.begin as usize))
                .ok_or_else(|| anyhow!("unknown function range"))?;

            while line.address.0 < range.end as usize {
                if line.is_stmt {
                    let load_addr = line.address.relocate(self.debugee.mapping_offset());
                    if line.address != current_line.address
                        && self
                            .breakpoints
                            .get(&PCValue::Relocated(load_addr))
                            .is_none()
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
            .try_for_each(|load_addr| self.set_breakpoint(PCValue::Relocated(load_addr)))?;

        if let Some(ret_addr) = uw::return_addr(self.debugee.threads_ctl.thread_in_focus())? {
            if self
                .breakpoints
                .get(&PCValue::Relocated(ret_addr))
                .is_none()
            {
                self.set_breakpoint(PCValue::Relocated(ret_addr))?;
                to_delete.push(ret_addr);
            }
        }

        self.continue_execution()?;

        to_delete
            .into_iter()
            .try_for_each(|addr| self.remove_breakpoint(PCValue::Relocated(addr)))?;

        Ok(())
    }

    pub fn set_breakpoint_at_fn(&mut self, name: &str) -> anyhow::Result<()> {
        let func = self
            .debugee
            .dwarf
            .find_function_by_name(name)
            .ok_or_else(|| anyhow!("function not found"))?;

        // todo find range with lowes begin
        let low_pc = func.die.base_attributes.ranges[0].begin;
        let entry = self
            .debugee
            .dwarf
            .find_place_from_pc(GlobalAddress(low_pc as usize))
            .ok_or_else(|| anyhow!("invalid function entry"))?
            // TODO skip prologue smarter
            .next()
            .ok_or_else(|| anyhow!("invalid function entry"))?;

        let addr = if self.debugee.in_progress {
            PCValue::Relocated(entry.address.relocate(self.debugee.mapping_offset()))
        } else {
            PCValue::Global(entry.address)
        };

        self.set_breakpoint(addr)
    }

    pub fn set_breakpoint_at_line(&mut self, fine_name: &str, line: u64) -> anyhow::Result<()> {
        if let Some(place) = self.debugee.dwarf.find_stmt_line(fine_name, line) {
            let addr = if self.debugee.in_progress {
                PCValue::Relocated(place.address.relocate(self.debugee.mapping_offset()))
            } else {
                PCValue::Global(place.address)
            };

            self.set_breakpoint(addr)?;
        }
        Ok(())
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
                        self.debugee.threads_ctl.thread_in_focus(),
                        type_decl,
                        known_parent_fn.or_else(|| var.assume_parent_function()),
                        self.debugee.mapping_addr?,
                    )
                });

                VariableIR::new(
                    &EvaluationContext {
                        unit: var.unit,
                        pid: self.debugee.threads_ctl.thread_in_focus(),
                    },
                    var.die.base_attributes.name.clone(),
                    mb_value,
                    mb_type,
                )
            })
            .collect();
        Ok(vars)
    }

    // Read all local variables from current thread.
    pub fn read_local_variables(&self) -> anyhow::Result<Vec<VariableIR>> {
        disable_when_not_stared!(self);

        let pc = self
            .get_current_thread_pc()?
            .to_global(self.debugee.mapping_offset());
        let current_func = self
            .debugee
            .dwarf
            .find_function_by_pc(pc)
            .ok_or_else(|| anyhow!("not in function"))?;
        let vars = current_func.find_variables(pc);
        self.variables_into_variable_ir(&vars, Some(current_func))
    }

    // Read any variable from current thread.
    pub fn read_variable(&self, name: &str) -> anyhow::Result<Vec<VariableIR>> {
        disable_when_not_stared!(self);

        let vars = self.debugee.dwarf.find_variables(name);
        self.variables_into_variable_ir(&vars, None)
    }

    pub fn get_register_value(&self, register_name: &str) -> anyhow::Result<u64> {
        disable_when_not_stared!(self);

        Ok(get_register_value(
            self.debugee.threads_ctl.thread_in_focus(),
            get_register_from_name(register_name)?,
        )?)
    }

    pub fn set_register_value(&self, register_name: &str, val: u64) -> anyhow::Result<()> {
        disable_when_not_stared!(self);

        Ok(set_register_value(
            self.debugee.threads_ctl.thread_in_focus(),
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
