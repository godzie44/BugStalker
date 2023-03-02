pub mod address;
mod breakpoint;
mod code;
pub mod command;
mod debugee;
pub mod register;
pub mod rust;
mod utils;
pub mod uw;
pub mod variable;

pub use debugee::dwarf::parser::unit::Place;
pub use debugee::dwarf::r#type::TypeDeclaration;
pub use debugee::ThreadDump;

use crate::debugger::address::{GlobalAddress, PCValue, RelocatedAddress};
use crate::debugger::breakpoint::Breakpoint;
use crate::debugger::command::expression::SelectPlan;
use crate::debugger::debugee::dwarf::r#type::TypeCache;
use crate::debugger::debugee::dwarf::{AsAllocatedValue, ContextualDieRef, RegisterDump, Symbol};
use crate::debugger::debugee::flow::{ControlFlow, DebugeeEvent};
use crate::debugger::debugee::{dwarf, Debugee, ExecutionStatus, FrameInfo, Location};
use crate::debugger::register::{get_register_from_name, get_register_value, set_register_value};
use crate::debugger::uw::Backtrace;
use crate::debugger::variable::VariableIR;
use crate::weak_error;
use anyhow::anyhow;
use nix::libc::{c_int, c_void, uintptr_t};
use nix::sys;
use nix::sys::signal;
use nix::sys::signal::Signal;
use nix::sys::wait::waitpid;
use nix::unistd::Pid;
use object::Object;
use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::ffi::c_long;
use std::path::Path;
use std::{fs, mem, u64};

pub trait EventHook {
    fn on_trap(&self, pc: RelocatedAddress, place: Option<Place>) -> anyhow::Result<()>;
    fn on_signal(&self, signo: c_int, code: c_int);
    fn on_exit(&self, code: i32);
}

macro_rules! disable_when_not_stared {
    ($this: expr) => {
        use anyhow::bail;
        if $this.debugee.execution_status != ExecutionStatus::InProgress {
            bail!("The program is not being started.")
        }
    };
}

/// Main structure of bug-stalker, control debugee state and provides application functionality.
pub struct Debugger {
    /// Debugee static/runtime state and control flow.
    debugee: Debugee,
    /// Active and non-active breakpoint list.
    breakpoints: HashMap<PCValue, Breakpoint>,
    /// Type declaration cache.
    type_cache: RefCell<TypeCache>,
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

        let entry_point = GlobalAddress::from(object.entry());
        let breakpoints = HashMap::from([(
            PCValue::Global(entry_point),
            Breakpoint::new(PCValue::Global(entry_point), pid),
        )]);

        Ok(Self {
            breakpoints,
            hooks: Box::new(hooks),
            type_cache: RefCell::default(),
            debugee: Debugee::new_non_running(program_path, pid, &object)?,
        })
    }

    fn continue_execution(&mut self) -> anyhow::Result<()> {
        self.step_over_breakpoint()?;

        loop {
            let event = self.debugee.control_flow_tick()?;
            match event {
                DebugeeEvent::DebugeeExit(code) => {
                    self.hooks.on_exit(code);
                    break;
                }
                DebugeeEvent::DebugeeStart => {
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
                }
                DebugeeEvent::AtEntryPoint(_) => {
                    self.step_over_breakpoint()?;
                }
                DebugeeEvent::TrapTrace | DebugeeEvent::NoSuchProcess(_) => {
                    break;
                }
                DebugeeEvent::Breakpoint(_, current_pc) => {
                    let offset_pc = current_pc.into_global(self.debugee.mapping_offset());
                    self.hooks
                        .on_trap(current_pc, self.debugee.dwarf.find_place_from_pc(offset_pc))?;
                    break;
                }
                DebugeeEvent::OsSignal(info, _) => {
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

    pub fn frame_info(&self, tid: Pid) -> anyhow::Result<FrameInfo> {
        disable_when_not_stared!(self);

        self.debugee.frame_info(self.debugee.thread_stop_at(tid)?)
    }

    pub fn step_into(&self) -> anyhow::Result<()> {
        disable_when_not_stared!(self);
        self.step_in()?;

        let location = self.current_thread_stop_at()?;
        self.hooks.on_trap(
            location.pc,
            self.debugee.dwarf.find_place_from_pc(location.global_pc),
        )
    }

    pub fn stepi(&self) -> anyhow::Result<()> {
        disable_when_not_stared!(self);
        self.single_step_instruction()?;
        let location = self.current_thread_stop_at()?;
        self.hooks.on_trap(
            location.pc,
            self.debugee.dwarf.find_place_from_pc(location.global_pc),
        )
    }

    pub fn thread_state(&self) -> anyhow::Result<Vec<ThreadDump>> {
        disable_when_not_stared!(self);
        self.debugee.thread_state()
    }

    pub fn backtrace(&self, pid: Pid) -> anyhow::Result<Backtrace> {
        disable_when_not_stared!(self);
        Ok(uw::backtrace(pid)?)
    }

    pub fn set_breakpoint(&mut self, addr: PCValue) -> anyhow::Result<()> {
        // todo make method idempotence
        let brkpt = Breakpoint::new(addr, self.debugee.threads_ctl().proc_pid());
        if self.debugee.execution_status == ExecutionStatus::InProgress {
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
            self.debugee.threads_ctl().proc_pid(),
            addr,
            read_n,
        )?)
    }

    pub fn write_memory(&self, addr: uintptr_t, value: uintptr_t) -> anyhow::Result<()> {
        disable_when_not_stared!(self);
        unsafe {
            Ok(sys::ptrace::write(
                self.debugee.threads_ctl().proc_pid(),
                addr as *mut c_void,
                value as *mut c_void,
            )?)
        }
    }

    pub fn current_thread_stop_at(&self) -> nix::Result<Location> {
        self.debugee.current_thread_stop_at()
    }

    fn step_over_breakpoint(&self) -> anyhow::Result<()> {
        // cannot use debugee::Location mapping offset may be not init yet
        let pid = self.debugee.thread_in_focus();
        let pc = self.debugee.control_flow.thread_pc(pid)?;
        let mb_brkpt = self.breakpoints.get(&PCValue::Relocated(pc));
        if let Some(brkpt) = mb_brkpt {
            if brkpt.is_enabled() {
                brkpt.disable()?;
                ControlFlow::thread_step(pid)?;
                brkpt.enable()?;
            }
        }
        Ok(())
    }

    fn single_step_instruction(&self) -> anyhow::Result<()> {
        let loc = self.current_thread_stop_at()?;
        if self.breakpoints.get(&PCValue::Relocated(loc.pc)).is_some() {
            self.step_over_breakpoint()
        } else {
            ControlFlow::thread_step(loc.pid)?;
            Ok(())
        }
    }

    pub fn step_out(&mut self) -> anyhow::Result<()> {
        disable_when_not_stared!(self);
        if let Some(ret_addr) = uw::return_addr(self.debugee.thread_in_focus())? {
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

        let location = self.current_thread_stop_at()?;
        let place = self
            .debugee
            .dwarf
            .find_place_from_pc(location.global_pc)
            .ok_or_else(|| anyhow!("not in debug frame (may be program not started?)"))?;

        while place
            == self
                .debugee
                .dwarf
                .find_place_from_pc(self.current_thread_stop_at()?.global_pc)
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
            .find_function_by_pc(self.current_thread_stop_at()?.global_pc)
            .ok_or_else(|| anyhow!("not in debug frame (may be program not started?)"))?;

        let mut to_delete = vec![];

        let current_line = self
            .debugee
            .dwarf
            .find_place_from_pc(self.current_thread_stop_at()?.global_pc)
            .ok_or_else(|| anyhow!("current line not found"))?;

        let mut breakpoints_range = vec![];

        for range in func.die.base_attributes.ranges.iter() {
            let mut line = self
                .debugee
                .dwarf
                .find_place_from_pc(GlobalAddress::from(range.begin))
                .ok_or_else(|| anyhow!("unknown function range"))?;

            while u64::from(line.address) < range.end {
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

        if let Some(ret_addr) = uw::return_addr(self.debugee.thread_in_focus())? {
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
            .find_place_from_pc(GlobalAddress::from(low_pc))
            .ok_or_else(|| anyhow!("invalid function entry"))?
            // TODO skip prologue smarter
            .next()
            .ok_or_else(|| anyhow!("invalid function entry"))?;

        let addr = if self.debugee.execution_status == ExecutionStatus::InProgress {
            PCValue::Relocated(entry.address.relocate(self.debugee.mapping_offset()))
        } else {
            PCValue::Global(entry.address)
        };

        self.set_breakpoint(addr)
    }

    pub fn set_breakpoint_at_line(&mut self, fine_name: &str, line: u64) -> anyhow::Result<()> {
        if let Some(place) = self.debugee.dwarf.find_stmt_line(fine_name, line) {
            let addr = if self.debugee.execution_status == ExecutionStatus::InProgress {
                PCValue::Relocated(place.address.relocate(self.debugee.mapping_offset()))
            } else {
                PCValue::Global(place.address)
            };

            self.set_breakpoint(addr)?;
        }
        Ok(())
    }

    fn variables_into_variable_ir<D: AsAllocatedValue>(
        &self,
        location: Location,
        vars: &[ContextualDieRef<D>],
        select_plan: SelectPlan,
    ) -> anyhow::Result<Vec<VariableIR>> {
        let mut type_cache = self.type_cache.borrow_mut();

        Ok(vars
            .iter()
            .filter_map(|var| {
                let var_name = var.die.name();
                let mb_type = var
                    .die
                    .type_ref()
                    .and_then(|type_ref| match type_cache.entry((var.unit.id, type_ref)) {
                        Entry::Occupied(o) => Some(&*o.into_mut()),
                        Entry::Vacant(v) => var.r#type().map(|t| &*v.insert(t)),
                    })
                    .ok_or(anyhow!(
                        "unknown type for variable {name}",
                        name = var_name.unwrap_or_default()
                    ));
                let r#type = weak_error!(mb_type)?;
                let mb_value = var.read_value_at_location(location, &self.debugee, r#type);

                let evaluator = var.unit.evaluator(&self.debugee);
                let parser = variable::VariableParser::new(r#type);
                let evaluation_context = &dwarf::r#type::EvaluationContext {
                    evaluator: &evaluator,
                    pid: location.pid,
                };

                let var = parser.parse(
                    evaluation_context,
                    variable::VariableIdentity::new(var.namespaces(), var_name.map(String::from)),
                    mb_value,
                );

                var.apply_select_plan(
                    evaluation_context,
                    &variable::VariableParser::new(r#type),
                    &select_plan,
                )
            })
            .collect())
    }

    // Read all local variables from current thread.
    pub fn read_local_variables(&self) -> anyhow::Result<Vec<VariableIR>> {
        disable_when_not_stared!(self);

        let location = self.current_thread_stop_at()?;
        let current_func = self
            .debugee
            .dwarf
            .find_function_by_pc(location.global_pc)
            .ok_or_else(|| anyhow!("not in function"))?;
        let vars = current_func.local_variables(location.global_pc);
        self.variables_into_variable_ir(location, &vars, SelectPlan::empty())
    }

    // Read any variable from current thread.
    pub fn read_variable(&self, select_plan: SelectPlan) -> anyhow::Result<Vec<VariableIR>> {
        disable_when_not_stared!(self);
        let location = self.current_thread_stop_at()?;
        let variable_name = select_plan
            .base_variable_name()
            .ok_or(anyhow!("invalid select expression"))?;
        let vars = self.debugee.dwarf.find_variables(location, variable_name);
        self.variables_into_variable_ir(self.current_thread_stop_at()?, &vars, select_plan)
    }

    // Read current function parameters.
    pub fn read_arguments(&self) -> anyhow::Result<Vec<VariableIR>> {
        disable_when_not_stared!(self);

        let location = self.current_thread_stop_at()?;
        let current_func = self
            .debugee
            .dwarf
            .find_function_by_pc(location.global_pc)
            .ok_or_else(|| anyhow!("not in function"))?;
        let params = current_func.parameters();
        self.variables_into_variable_ir(location, &params, SelectPlan::empty())
    }

    // Read any argument from current function.
    pub fn read_argument(&self, select_plan: SelectPlan) -> anyhow::Result<Vec<VariableIR>> {
        disable_when_not_stared!(self);

        let arg_name = select_plan
            .base_variable_name()
            .ok_or(anyhow!("invalid select expression"))?;

        let location = self.current_thread_stop_at()?;
        let current_func = self
            .debugee
            .dwarf
            .find_function_by_pc(location.global_pc)
            .ok_or_else(|| anyhow!("not in function"))?;
        let params = current_func.parameters();
        let params = params
            .into_iter()
            .filter(|param| {
                param
                    .die
                    .base_attributes
                    .name
                    .as_ref()
                    .map(|n| n == arg_name)
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        self.variables_into_variable_ir(location, &params, select_plan)
    }

    pub fn get_register_value(&self, register_name: &str) -> anyhow::Result<u64> {
        disable_when_not_stared!(self);

        Ok(get_register_value(
            self.debugee.thread_in_focus(),
            get_register_from_name(register_name)?,
        )?)
    }

    pub fn current_thread_registers_at_pc(
        &self,
        pc: RelocatedAddress,
    ) -> anyhow::Result<RegisterDump> {
        self.get_registers(Location {
            pc,
            global_pc: pc.into_global(self.debugee.mapping_offset()),
            pid: self.debugee.thread_in_focus(),
        })
    }

    pub fn get_registers(&self, at_location: Location) -> anyhow::Result<RegisterDump> {
        disable_when_not_stared!(self);
        let current_location = self.current_thread_stop_at()?;
        self.debugee
            .dwarf
            .registers(&self.debugee, at_location, current_location)
    }

    pub fn set_register_value(&self, register_name: &str, val: u64) -> anyhow::Result<()> {
        disable_when_not_stared!(self);

        Ok(set_register_value(
            self.debugee.thread_in_focus(),
            get_register_from_name(register_name)?,
            val,
        )?)
    }
}

impl Drop for Debugger {
    fn drop(&mut self) {
        match self.debugee.execution_status {
            ExecutionStatus::Unload => {
                signal::kill(self.debugee.threads_ctl().proc_pid(), Signal::SIGKILL)
                    .expect("kill debugee");
                waitpid(self.debugee.threads_ctl().proc_pid(), None).expect("waiting child");
            }
            ExecutionStatus::InProgress => {
                self.debugee.threads_ctl().dump().iter().for_each(|thread| {
                    sys::ptrace::detach(thread.pid, None).expect("detach thread")
                });
                signal::kill(self.debugee.threads_ctl().proc_pid(), Signal::SIGKILL)
                    .expect("kill debugee");
                waitpid(self.debugee.threads_ctl().proc_pid(), None).expect("waiting child");
            }
            ExecutionStatus::Exited => {}
        }
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
