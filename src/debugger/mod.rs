pub mod address;
mod breakpoint;
mod code;
pub mod command;
mod debugee;
pub mod process;
pub mod register;
pub mod rust;
mod utils;
pub mod variable;

pub use debugee::dwarf::r#type::TypeDeclaration;
pub use debugee::dwarf::unit::Place;
pub use debugee::dwarf::unwind;
pub use debugee::ThreadSnapshot;

use crate::debugger::address::{Address, GlobalAddress, RelocatedAddress};
use crate::debugger::breakpoint::{Breakpoint, BrkptType};
use crate::debugger::debugee::dwarf::r#type::TypeCache;
use crate::debugger::debugee::dwarf::unit::PlaceOwned;
use crate::debugger::debugee::dwarf::unwind::libunwind;
use crate::debugger::debugee::dwarf::unwind::libunwind::Backtrace;
use crate::debugger::debugee::dwarf::{DwarfUnwinder, Symbol};
use crate::debugger::debugee::tracer::{StopReason, TraceContext};
use crate::debugger::debugee::{Debugee, ExecutionStatus, FrameInfo, Location};
use crate::debugger::process::{Child, Installed};
use crate::debugger::register::{DwarfRegisterMap, Register, RegisterMap};
use crate::debugger::variable::select::{Expression, VariableSelector};
use crate::debugger::variable::VariableIR;
use anyhow::anyhow;
use nix::libc::{c_void, uintptr_t};
use nix::sys;
use nix::sys::signal;
use nix::sys::signal::{Signal, SIGKILL};
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::Pid;
use object::Object;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_long;
use std::path::Path;
use std::str::FromStr;
use std::{fs, mem, u64};

/// Trait for the reverse interaction between the debugger and the user interface.
pub trait EventHook {
    /// Called when user defined breakpoint is reached.
    ///
    /// # Arguments
    ///
    /// * `pc`: address of instruction where breakpoint is reached
    /// * `place`: stop place information
    fn on_breakpoint(&self, pc: RelocatedAddress, place: Option<Place>) -> anyhow::Result<()>;

    /// Called when one of step commands is done.
    ///
    /// # Arguments
    ///
    /// * `pc`: address of instruction where breakpoint is reached
    /// * `place`: stop place information
    fn on_step(&self, pc: RelocatedAddress, place: Option<Place>) -> anyhow::Result<()>;

    /// Called when debugee receive a OS signal. Debugee is in signal-stop at this moment.
    ///
    /// # Arguments
    ///
    /// * `signal`: received OS signal
    fn on_signal(&self, signal: Signal);

    /// Called right after debugee exit.
    ///
    /// # Arguments
    ///
    /// * `code`: exit code
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
    /// Child process where debugee is running.
    process: Child<Installed>,
    /// Debugee static/runtime state and control flow.
    debugee: Debugee,
    /// Active and non-active breakpoint list.
    breakpoints: HashMap<Address, Breakpoint>,
    /// Type declaration cache.
    type_cache: RefCell<TypeCache>,
    /// Debugger interrupt with UI by EventHook trait.
    hooks: Box<dyn EventHook>,
}

impl Debugger {
    pub fn new(process: Child<Installed>, hooks: impl EventHook + 'static) -> anyhow::Result<Self> {
        let program = process.program.clone();
        let program_path = Path::new(&program);

        let file = fs::File::open(program_path)?;
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        let object = object::File::parse(&*mmap)?;

        let entry_point = GlobalAddress::from(object.entry());
        let breakpoints = HashMap::from([(
            Address::Global(entry_point),
            Breakpoint::new_entry_point(Address::Global(entry_point), process.pid()),
        )]);

        Ok(Self {
            debugee: Debugee::new_non_running(program_path, process.pid(), &object)?,
            process,
            breakpoints,
            hooks: Box::new(hooks),
            type_cache: RefCell::default(),
        })
    }

    fn continue_execution(&mut self) -> anyhow::Result<()> {
        self.step_over_breakpoint()?;

        loop {
            let event = self
                .debugee
                .trace_until_stop(TraceContext::new(&self.breakpoints.values().collect()))?;
            match event {
                StopReason::DebugeeExit(code) => {
                    self.hooks.on_exit(code);
                    break;
                }
                StopReason::DebugeeStart => {
                    let mut brkpts_to_reloc = HashMap::with_capacity(self.breakpoints.len());
                    let keys = self.breakpoints.keys().copied().collect::<Vec<_>>();
                    for k in keys {
                        if let Address::Global(addr) = k {
                            brkpts_to_reloc.insert(addr, self.breakpoints.remove(&k).unwrap());
                        }
                    }
                    for (addr, mut brkpt) in brkpts_to_reloc {
                        brkpt.addr =
                            Address::Relocated(addr.relocate(self.debugee.mapping_offset()));
                        self.breakpoints.insert(brkpt.addr, brkpt);
                    }
                    self.breakpoints
                        .iter()
                        .try_for_each(|(_, brkpt)| brkpt.enable())?;

                    debug_assert!(self
                        .breakpoints
                        .iter()
                        .all(|(addr, _)| matches!(addr, Address::Relocated(_))));
                }
                StopReason::NoSuchProcess(_) => {
                    break;
                }
                StopReason::Breakpoint(_, current_pc) => {
                    if let Some(bp) = self.breakpoints.get(&Address::Relocated(current_pc)) {
                        match bp.r#type() {
                            BrkptType::EntryPoint => {
                                self.step_over_breakpoint()?;
                                continue;
                            }
                            BrkptType::UserDefined => {
                                let pc = current_pc.into_global(self.debugee.mapping_offset());
                                self.hooks.on_breakpoint(
                                    current_pc,
                                    self.debugee.dwarf.find_place_from_pc(pc),
                                )?;
                                break;
                            }
                            BrkptType::Temporary => {
                                break;
                            }
                        }
                    }
                }
                StopReason::SignalStop(_, sign) => {
                    // todo inject signal on next continue
                    self.hooks.on_signal(sign);
                    break;
                }
            }
        }

        Ok(())
    }

    /// Restart debugee by recreating debugee process, save all user defined breakpoints.
    fn restart_debugee(&mut self) -> anyhow::Result<()> {
        self.breakpoints
            .iter()
            .try_for_each(|(_, bp)| bp.disable())?;

        let proc_pid = self.debugee.tracee_ctl().proc_pid();
        signal::kill(proc_pid, SIGKILL)?;
        _ = self
            .debugee
            .tracer_mut()
            .resume(TraceContext::new(&self.breakpoints.values().collect()));

        self.process = self.process.install()?;

        let new_debugee = self.debugee.extend(self.process.pid());
        _ = mem::replace(&mut self.debugee, new_debugee);

        // breakpoints will be enabled later, when StopReason::DebugeeStart state is reached
        self.breakpoints.iter_mut().for_each(|(_, bp)| {
            bp.pid = self.process.pid();
        });

        self.continue_execution()
    }

    /// Start debugee.
    pub fn start_debugee(&mut self) -> anyhow::Result<()> {
        if self.debugee.execution_status != ExecutionStatus::InProgress {
            return self.continue_execution();
        }
        Ok(())
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

        self.debugee.frame_info(
            self.debugee
                .tracee_ctl()
                .tracee_ensure(tid)
                .location(&self.debugee)?,
        )
    }

    pub fn step_into(&mut self) -> anyhow::Result<()> {
        disable_when_not_stared!(self);
        self.step_in()?;

        let location = self.current_thread_stop_at()?;
        self.hooks.on_step(
            location.pc,
            self.debugee.dwarf.find_place_from_pc(location.global_pc),
        )
    }

    pub fn stepi(&mut self) -> anyhow::Result<()> {
        disable_when_not_stared!(self);
        self.single_step_instruction()?;
        let location = self.current_thread_stop_at()?;
        self.hooks.on_step(
            location.pc,
            self.debugee.dwarf.find_place_from_pc(location.global_pc),
        )
    }

    pub fn thread_state(&self) -> anyhow::Result<Vec<ThreadSnapshot>> {
        disable_when_not_stared!(self);
        self.debugee.thread_state()
    }

    pub fn backtrace(&self, pid: Pid) -> anyhow::Result<Backtrace> {
        disable_when_not_stared!(self);
        Ok(libunwind::unwind(pid)?)
    }

    fn add_breakpoint(&mut self, brkpt: Breakpoint) -> anyhow::Result<()> {
        let addr = brkpt.addr;
        if self.debugee.execution_status == ExecutionStatus::InProgress {
            if let Some(existed) = self.breakpoints.get(&addr) {
                existed.disable()?;
            }
            brkpt.enable()?;
        }
        self.breakpoints.insert(addr, brkpt);
        Ok(())
    }

    pub fn new_breakpoint(&mut self, addr: Address) -> anyhow::Result<()> {
        let brkpt = Breakpoint::new(addr, self.debugee.tracee_ctl().proc_pid());
        self.add_breakpoint(brkpt)
    }

    pub fn remove_breakpoint(&mut self, addr: Address) -> anyhow::Result<()> {
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
            self.debugee.tracee_ctl().proc_pid(),
            addr,
            read_n,
        )?)
    }

    pub fn write_memory(&self, addr: uintptr_t, value: uintptr_t) -> anyhow::Result<()> {
        disable_when_not_stared!(self);
        unsafe {
            Ok(sys::ptrace::write(
                self.debugee.tracee_ctl().proc_pid(),
                addr as *mut c_void,
                value as *mut c_void,
            )?)
        }
    }

    pub fn current_thread_stop_at(&self) -> nix::Result<Location> {
        self.debugee.tracee_in_focus().location(&self.debugee)
    }

    fn step_over_breakpoint(&mut self) -> anyhow::Result<()> {
        // cannot use debugee::Location mapping offset may be not init yet
        let tracee = self.debugee.tracee_in_focus();
        let mb_brkpt = self.breakpoints.get(&Address::Relocated(tracee.pc()?));
        let tracee_pid = tracee.pid;
        if let Some(brkpt) = mb_brkpt {
            if brkpt.is_enabled() {
                brkpt.disable()?;
                self.debugee.tracer_mut().single_step(
                    TraceContext::new(&self.breakpoints.values().collect()),
                    tracee_pid,
                )?;
                brkpt.enable()?;
            }
        }
        Ok(())
    }

    fn single_step_instruction(&mut self) -> anyhow::Result<()> {
        let loc = self.current_thread_stop_at()?;
        if self.breakpoints.get(&Address::Relocated(loc.pc)).is_some() {
            self.step_over_breakpoint()
        } else {
            self.debugee.tracer_mut().single_step(
                TraceContext::new(&self.breakpoints.values().collect()),
                loc.pid,
            )?;
            Ok(())
        }
    }

    pub fn step_out(&mut self) -> anyhow::Result<()> {
        disable_when_not_stared!(self);
        let location = self.current_thread_stop_at()?;
        if let Some(ret_addr) = libunwind::return_addr(location.pid)? {
            let brkpt_is_set = self
                .breakpoints
                .get(&Address::Relocated(ret_addr))
                .is_some();
            if brkpt_is_set {
                self.continue_execution()?;
            } else {
                self.add_breakpoint(Breakpoint::new_temporary(
                    Address::Relocated(ret_addr),
                    location.pid,
                ))?;
                self.continue_execution()?;
                self.remove_breakpoint(Address::Relocated(ret_addr))?;

                let new_location = self.current_thread_stop_at()?;
                self.hooks.on_step(
                    new_location.pc,
                    self.debugee
                        .dwarf
                        .find_place_from_pc(new_location.global_pc),
                )?;
            }
        }
        Ok(())
    }

    pub fn step_in(&mut self) -> anyhow::Result<()> {
        disable_when_not_stared!(self);

        // make instruction step but ignoring functions prolog
        // initial function must exists (do instruction steps until it's not)
        fn long_step(debugger: &mut Debugger) -> anyhow::Result<PlaceOwned> {
            loop {
                // initial step
                debugger.single_step_instruction()?;

                let mut location = debugger.current_thread_stop_at()?;
                let func = loop {
                    let dwarf = &debugger.debugee.dwarf;
                    if let Some(func) = dwarf.find_function_by_pc(location.global_pc) {
                        break func;
                    }
                    debugger.single_step_instruction()?;
                    location = debugger.current_thread_stop_at()?;
                };

                let prolog = func.prolog()?;
                // if pc in prolog range - step until function body is reached
                while debugger
                    .current_thread_stop_at()?
                    .global_pc
                    .in_range(&prolog)
                {
                    debugger.single_step_instruction()?;
                }

                if let Some(place) = debugger
                    .debugee
                    .dwarf
                    .find_exact_place_from_pc(debugger.current_thread_stop_at()?.global_pc)
                {
                    return Ok(place.to_owned());
                }
            }
        }

        let mut location = self.current_thread_stop_at()?;

        let start_place = loop {
            let dwarf = &self.debugee.dwarf;
            if let Some(place) = dwarf.find_place_from_pc(location.global_pc) {
                break place;
            }
            self.single_step_instruction()?;
            location = self.current_thread_stop_at()?;
        };

        let sp_file = start_place.file.to_path_buf();
        let sp_line = start_place.line_number;
        let start_cfa = self.debugee.dwarf.get_cfa(&self.debugee, location)?;

        loop {
            let next_place = long_step(self)?;
            if !next_place.is_stmt {
                continue;
            }
            let in_same_place = sp_file == next_place.file && sp_line == next_place.line_number;
            let next_cfa = self
                .debugee
                .dwarf
                .get_cfa(&self.debugee, self.current_thread_stop_at()?)?;

            // step is done if:
            // 1) we may step at same place in code but in another stack frame
            // 2) we step at another place in code (file + line)
            if start_cfa != next_cfa || !in_same_place {
                break;
            }
        }

        Ok(())
    }

    pub fn step_over(&mut self) -> anyhow::Result<()> {
        disable_when_not_stared!(self);

        let mut current_location = self.current_thread_stop_at()?;

        let func = loop {
            let dwarf = &self.debugee.dwarf;
            if let Some(func) = dwarf.find_function_by_pc(current_location.global_pc) {
                break func;
            }
            self.single_step_instruction()?;
            current_location = self.current_thread_stop_at()?;
        };

        let prolog = func.prolog()?;
        let dwarf = &self.debugee.dwarf;
        let inline_ranges = func.inline_ranges();

        let current_place = dwarf
            .find_place_from_pc(current_location.global_pc)
            .ok_or_else(|| anyhow!("current line not found"))?;

        let mut step_over_breakpoints = vec![];
        let mut to_delete = vec![];

        for range in func.ranges() {
            let mut place = func
                .unit()
                .find_place_by_pc(GlobalAddress::from(range.begin))
                .ok_or_else(|| anyhow!("unknown function range"))?;

            while place.address.in_range(range) {
                // skip places in function prolog
                if place.address.in_range(&prolog) {
                    match place.next() {
                        None => break,
                        Some(n) => place = n,
                    }
                    continue;
                }

                // guard from step at inlined function body
                let in_inline_range = inline_ranges
                    .iter()
                    .any(|inline_range| place.address.in_range(inline_range));

                if !in_inline_range
                    && place.is_stmt
                    && place.address != current_place.address
                    && place.line_number != current_place.line_number
                {
                    let load_addr = place.address.relocate(self.debugee.mapping_offset());
                    if !self
                        .breakpoints
                        .contains_key(&Address::Relocated(load_addr))
                    {
                        step_over_breakpoints.push(load_addr);
                        to_delete.push(load_addr);
                    }
                }

                match place.next() {
                    None => break,
                    Some(n) => place = n,
                }
            }
        }

        step_over_breakpoints
            .into_iter()
            .try_for_each(|load_addr| {
                self.add_breakpoint(Breakpoint::new_temporary(
                    Address::Relocated(load_addr),
                    current_location.pid,
                ))
            })?;

        let return_addr = libunwind::return_addr(current_location.pid)?;
        if let Some(ret_addr) = return_addr {
            if !self.breakpoints.contains_key(&Address::Relocated(ret_addr)) {
                self.add_breakpoint(Breakpoint::new_temporary(
                    Address::Relocated(ret_addr),
                    current_location.pid,
                ))?;
                to_delete.push(ret_addr);
            }
        }

        self.continue_execution()?;

        to_delete
            .into_iter()
            .try_for_each(|addr| self.remove_breakpoint(Address::Relocated(addr)))?;

        // if a step is taken outside then do single `step into`
        let mut new_location = self.current_thread_stop_at()?;
        if Some(new_location.pc) == return_addr {
            self.step_in()?;
            new_location = self.current_thread_stop_at()?;
        }

        self.hooks.on_step(
            new_location.pc,
            self.debugee
                .dwarf
                .find_place_from_pc(new_location.global_pc),
        )?;

        Ok(())
    }

    fn address_for_fn(&self, name: &str) -> anyhow::Result<Address> {
        let dwarf = &self.debugee.dwarf;
        let func = dwarf
            .find_function_by_name(name)
            .ok_or_else(|| anyhow!("function not found"))?;
        let place = func.prolog_end_place()?;

        Ok(
            if self.debugee.execution_status == ExecutionStatus::InProgress {
                Address::Relocated(place.address.relocate(self.debugee.mapping_offset()))
            } else {
                Address::Global(place.address)
            },
        )
    }

    pub fn set_breakpoint_at_fn(&mut self, name: &str) -> anyhow::Result<()> {
        self.new_breakpoint(self.address_for_fn(name)?)
    }

    pub fn remove_breakpoint_at_fn(&mut self, name: &str) -> anyhow::Result<()> {
        self.remove_breakpoint(self.address_for_fn(name)?)
    }

    fn address_for_line(&mut self, fine_name: &str, line: u64) -> Option<Address> {
        if let Some(place) = self.debugee.dwarf.find_stmt_line(fine_name, line) {
            let addr = if self.debugee.execution_status == ExecutionStatus::InProgress {
                Address::Relocated(place.address.relocate(self.debugee.mapping_offset()))
            } else {
                Address::Global(place.address)
            };
            return Some(addr);
        }
        None
    }

    pub fn set_breakpoint_at_line(&mut self, fine_name: &str, line: u64) -> anyhow::Result<()> {
        if let Some(addr) = self.address_for_line(fine_name, line) {
            self.new_breakpoint(addr)?;
        }
        Ok(())
    }

    pub fn remove_breakpoint_at_line(&mut self, fine_name: &str, line: u64) -> anyhow::Result<()> {
        if let Some(addr) = self.address_for_line(fine_name, line) {
            self.remove_breakpoint(addr)?;
        }
        Ok(())
    }

    // Reads all local variables from current function in current thread.
    pub fn read_local_variables(&self) -> anyhow::Result<Vec<VariableIR>> {
        disable_when_not_stared!(self);

        let evaluator = variable::select::SelectExpressionEvaluator::new(
            self,
            Expression::Variable(VariableSelector::Any),
        )?;
        evaluator.evaluate()
    }

    // Reads any variable from the current thread, uses a select expression to filter variables
    // and fetch their properties (such as structure fields or array elements).
    pub fn read_variable(&self, select_expr: Expression) -> anyhow::Result<Vec<VariableIR>> {
        disable_when_not_stared!(self);
        let evaluator = variable::select::SelectExpressionEvaluator::new(self, select_expr)?;
        evaluator.evaluate()
    }

    // Reads any argument from the current function, uses a select expression to filter variables
    // and fetch their properties (such as structure fields or array elements).
    pub fn read_argument(&self, select_expr: Expression) -> anyhow::Result<Vec<VariableIR>> {
        disable_when_not_stared!(self);
        let evaluator = variable::select::SelectExpressionEvaluator::new(self, select_expr)?;
        evaluator.evaluate_on_arguments()
    }

    pub fn get_register_value(&self, register_name: &str) -> anyhow::Result<u64> {
        disable_when_not_stared!(self);

        Ok(RegisterMap::current(self.debugee.tracee_in_focus().pid)?
            .value(Register::from_str(register_name)?))
    }

    pub fn current_thread_registers_at_pc(
        &self,
        pc: RelocatedAddress,
    ) -> anyhow::Result<DwarfRegisterMap> {
        disable_when_not_stared!(self);
        let unwinder = DwarfUnwinder::new(&self.debugee);

        Ok(unwinder
            .context_for(Location {
                pc,
                global_pc: pc.into_global(self.debugee.mapping_offset()),
                pid: self.debugee.tracee_in_focus().pid,
            })?
            .ok_or(anyhow!("fetch register fail"))?
            .registers())
    }

    pub fn set_register_value(&self, register_name: &str, val: u64) -> anyhow::Result<()> {
        disable_when_not_stared!(self);

        let mut map = RegisterMap::current(self.debugee.tracee_in_focus().pid)?;
        map.update(Register::try_from(register_name)?, val);
        Ok(map.persist(self.debugee.tracee_in_focus().pid)?)
    }
}

impl Drop for Debugger {
    fn drop(&mut self) {
        match self.debugee.execution_status {
            ExecutionStatus::Unload => {
                signal::kill(self.debugee.tracee_ctl().proc_pid(), Signal::SIGKILL)
                    .expect("kill debugee");
                waitpid(self.debugee.tracee_ctl().proc_pid(), None).expect("waiting child");
            }
            ExecutionStatus::InProgress => {
                self.breakpoints
                    .iter()
                    .try_for_each(|(_, bp)| bp.disable())
                    .expect("stop debugee");

                let current_tids: Vec<Pid> = self
                    .debugee
                    .tracee_ctl()
                    .snapshot()
                    .iter()
                    .map(|t| t.pid)
                    .collect();

                // todo currently ok only if all threads in group stop
                // continue all threads with SIGSTOP
                current_tids.iter().for_each(|tid| {
                    sys::ptrace::cont(*tid, Signal::SIGSTOP).expect("cont debugee");
                });
                current_tids.iter().for_each(|tid| {
                    waitpid(*tid, None).expect("waiting debugee");
                });
                // detach ptrace
                current_tids.iter().for_each(|tid| {
                    sys::ptrace::detach(*tid, None).expect("detach debugee");
                });
                // kill debugee process
                signal::kill(self.debugee.tracee_ctl().proc_pid(), Signal::SIGKILL)
                    .expect("kill debugee");
                let wait_result =
                    waitpid(self.debugee.tracee_ctl().proc_pid(), None).expect("waiting debugee");

                debug_assert!(matches!(
                    wait_result,
                    WaitStatus::Signaled(_, Signal::SIGKILL, _)
                ));
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
