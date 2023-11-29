pub mod address;
mod breakpoint;
mod code;
mod debugee;
mod error;
pub mod process;
pub mod register;
pub mod rust;
mod step;
mod utils;
pub mod variable;

pub use breakpoint::BreakpointView;
pub use breakpoint::BreakpointViewOwned;
pub use debugee::dwarf::r#type::TypeDeclaration;
pub use debugee::dwarf::unit::FunctionDie;
pub use debugee::dwarf::unit::PlaceDescriptor;
pub use debugee::dwarf::unit::PlaceDescriptorOwned;
pub use debugee::dwarf::unwind;
pub use debugee::dwarf::Symbol;
pub use debugee::tracee::Tracee;
pub use debugee::FrameInfo;
pub use debugee::FunctionAssembly;
pub use debugee::RegionInfo;
pub use debugee::ThreadSnapshot;
pub use error::Error;

use crate::debugger::address::{Address, GlobalAddress, RelocatedAddress};
use crate::debugger::breakpoint::{Breakpoint, BreakpointRegistry, BrkptType, UninitBreakpoint};
use crate::debugger::debugee::dwarf::r#type::TypeCache;
use crate::debugger::debugee::dwarf::unwind::Backtrace;
use crate::debugger::debugee::dwarf::DwarfUnwinder;
use crate::debugger::debugee::tracer::{StopReason, TraceContext};
use crate::debugger::debugee::{Debugee, ExecutionStatus, Location};
use crate::debugger::error::Error::{
    FrameNotFound, Hook, ProcessNotStarted, Ptrace, RegisterNameNotFound, UnwindNoContext,
};
use crate::debugger::process::{Child, Installed};
use crate::debugger::register::{DwarfRegisterMap, Register, RegisterMap};
use crate::debugger::step::StepResult;
use crate::debugger::variable::select::{Expression, VariableSelector};
use crate::debugger::variable::VariableIR;
use crate::debugger::Error::Syscall;
use crate::{print_warns, weak_error};
use nix::libc::{c_void, uintptr_t};
use nix::sys;
use nix::sys::signal;
use nix::sys::signal::{Signal, SIGKILL};
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::Pid;
use object::Object;
use regex::Regex;
use std::cell::RefCell;
use std::ffi::c_long;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::{fs, mem, u64};

/// Trait for the reverse interaction between the debugger and the user interface.
pub trait EventHook {
    /// Called when user defined breakpoint is reached.
    ///
    /// # Arguments
    ///
    /// * `pc`: address of instruction where breakpoint is reached
    /// * `num`: breakpoint number
    /// * `place`: stop place information
    /// * `function`: function debug information entry
    fn on_breakpoint(
        &self,
        pc: RelocatedAddress,
        num: u32,
        place: Option<PlaceDescriptor>,
        function: Option<&FunctionDie>,
    ) -> anyhow::Result<()>;

    /// Called when one of step commands is done.
    ///
    /// # Arguments
    ///
    /// * `pc`: address of instruction where breakpoint is reached
    /// * `place`: stop place information
    /// * `function`: function debug information entry
    fn on_step(
        &self,
        pc: RelocatedAddress,
        place: Option<PlaceDescriptor>,
        function: Option<&FunctionDie>,
    ) -> anyhow::Result<()>;

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

    /// Called single time for each debugee process (on start or after reinstall).
    ///
    /// # Arguments
    ///
    /// * `pid`: debugee process pid
    fn on_process_install(&self, pid: Pid);
}

pub struct DoNothingHook {}

impl EventHook for DoNothingHook {
    fn on_breakpoint(
        &self,
        _: RelocatedAddress,
        _: u32,
        _: Option<PlaceDescriptor>,
        _: Option<&FunctionDie>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn on_step(
        &self,
        _: RelocatedAddress,
        _: Option<PlaceDescriptor>,
        _: Option<&FunctionDie>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn on_signal(&self, _: Signal) {}

    fn on_exit(&self, _: i32) {}

    fn on_process_install(&self, _: Pid) {}
}

macro_rules! disable_when_not_stared {
    ($this: expr) => {
        if !$this.debugee.is_in_progress() {
            return Err(ProcessNotStarted);
        }
    };
}

/// Exploration context. Contains current explored thread and program counter.
/// May changed by user (by `thread` or `frame` command)
/// or by debugger (at breakpoints, after steps, etc.).
#[derive(Clone, Debug)]
pub struct ExplorationContext {
    focus_location: Location,
    focus_frame: u32,
}

impl ExplorationContext {
    /// Create new context with known thread but without known program counter value.
    /// It is useful when debugee not started yet or restarted.
    ///
    /// # Arguments
    ///
    /// * `pid`: thread id
    pub fn new_non_running(pid: Pid) -> ExplorationContext {
        Self {
            focus_location: Location {
                pc: 0_u64.into(),
                global_pc: 0_u64.into(),
                pid,
            },
            focus_frame: 0,
        }
    }

    /// Create new context.
    pub fn new(location: Location, frame_num: u32) -> Self {
        Self {
            focus_location: location,
            focus_frame: frame_num,
        }
    }

    #[inline(always)]
    pub fn location(&self) -> Location {
        self.focus_location
    }

    #[inline(always)]
    pub fn frame(&self) -> u32 {
        self.focus_frame
    }

    #[inline(always)]
    pub fn pid_on_focus(&self) -> Pid {
        self.location().pid
    }
}

/// Main structure of bug-stalker, control debugee state and provides application functionality.
pub struct Debugger {
    /// Child process where debugee is running.
    process: Child<Installed>,
    /// Debugee static/runtime state and control flow.
    debugee: Debugee,
    /// Active and non-active breakpoints lists.
    breakpoints: BreakpointRegistry,
    /// Type declaration cache.
    type_cache: RefCell<TypeCache>,
    /// Debugger interrupt with UI by EventHook trait.
    hooks: Box<dyn EventHook>,
    /// Current exploration context.
    expl_context: ExplorationContext,
}

impl Debugger {
    pub fn new(process: Child<Installed>, hooks: impl EventHook + 'static) -> anyhow::Result<Self> {
        let program_path = Path::new(&process.program);

        let file = fs::File::open(program_path)?;
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        let object = object::File::parse(&*mmap)?;

        let entry_point = GlobalAddress::from(object.entry());
        let mut breakpoints = BreakpointRegistry::default();
        breakpoints.add_uninit(UninitBreakpoint::new_entry_point(
            None::<PathBuf>,
            Address::Global(entry_point),
            process.pid(),
        ));

        let process_id = process.pid();
        hooks.on_process_install(process_id);

        Ok(Self {
            debugee: Debugee::new_non_running(program_path, process_id, &object)?,
            process,
            breakpoints,
            hooks: Box::new(hooks),
            type_cache: RefCell::default(),
            expl_context: ExplorationContext::new_non_running(process_id),
        })
    }

    pub fn process(&self) -> &Child<Installed> {
        &self.process
    }

    pub fn set_hook(&mut self, hooks: impl EventHook + 'static) {
        self.hooks = Box::new(hooks);
    }

    /// Return last set exploration context.
    #[inline(always)]
    pub fn exploration_ctx(&self) -> &ExplorationContext {
        &self.expl_context
    }

    /// Update current program counters for current in focus thread.
    fn expl_ctx_update_location(&mut self) -> Result<&ExplorationContext, Error> {
        let old_ctx = self.exploration_ctx();
        self.expl_context = ExplorationContext::new(
            self.debugee
                .get_tracee_ensure(old_ctx.pid_on_focus())
                .location(&self.debugee)?,
            0,
        );
        Ok(&self.expl_context)
    }

    /// Restore frame from user defined to real.
    fn expl_ctx_restore_frame(&mut self) -> Result<&ExplorationContext, Error> {
        self.expl_ctx_update_location()
    }

    /// Change in focus thread and update program counters.
    ///
    /// # Arguments
    ///
    /// * `pid`: new in focus thread id
    fn expl_ctx_switch_thread(&mut self, pid: Pid) -> Result<&ExplorationContext, Error> {
        self.expl_context = ExplorationContext::new(
            self.debugee
                .get_tracee_ensure(pid)
                .location(&self.debugee)?,
            0,
        );
        Ok(&self.expl_context)
    }

    /// Continue debugee execution. Step over breakpoint if called at it.
    /// Return if breakpoint is reached or signal occurred or debugee exit.
    ///
    /// **! change exploration context**
    fn continue_execution(&mut self) -> Result<StopReason, Error> {
        if let Some(StopReason::SignalStop(pid, sign)) = self.step_over_breakpoint()? {
            self.hooks.on_signal(sign);
            return Ok(StopReason::SignalStop(pid, sign));
        }

        let stop_reason = loop {
            let event = self
                .debugee
                .trace_until_stop(TraceContext::new(&self.breakpoints.active_breakpoints()))?;
            match event {
                StopReason::DebugeeExit(code) => {
                    // ignore all possible errors on breakpoints disabling
                    _ = self.breakpoints.disable_all_breakpoints(&self.debugee);
                    self.hooks.on_exit(code);
                    break event;
                }
                StopReason::DebugeeStart => {
                    self.breakpoints.enable_entry_breakpoint(&self.debugee)?;
                    // no need to update expl context cause next stop been soon, on entry point
                }
                StopReason::NoSuchProcess(_) => {
                    return Err(ProcessNotStarted);
                }
                StopReason::Breakpoint(pid, current_pc) => {
                    self.expl_ctx_switch_thread(pid)?;

                    if let Some(bp) = self.breakpoints.get_enabled(current_pc) {
                        match bp.r#type() {
                            BrkptType::EntryPoint => {
                                print_warns!(self
                                    .breakpoints
                                    .enable_all_breakpoints(&self.debugee));

                                // rendezvous already available at this point
                                let brk = self.debugee.rendezvous().r_brk();
                                self.breakpoints.add_and_enable(Breakpoint::new_linker_map(
                                    brk,
                                    self.process.pid(),
                                ))?;

                                // ignore possible signals
                                while self.step_over_breakpoint()?.is_some() {}
                                continue;
                            }
                            BrkptType::LinkerMapFn => {
                                // ignore possible signals
                                while self.step_over_breakpoint()?.is_some() {}
                                print_warns!(self.refresh_deferred());
                                continue;
                            }
                            BrkptType::UserDefined => {
                                let pc = current_pc.into_global(&self.debugee)?;
                                let dwarf = self
                                    .debugee
                                    .debug_info(self.exploration_ctx().location().pc)?;
                                let place = weak_error!(dwarf.find_place_from_pc(pc)).flatten();
                                let func = weak_error!(dwarf.find_function_by_pc(pc))
                                    .flatten()
                                    .map(|f| f.die);
                                self.hooks
                                    .on_breakpoint(current_pc, bp.number(), place, func)
                                    .map_err(Hook)?;
                                break event;
                            }
                            BrkptType::Temporary => {
                                break event;
                            }
                        }
                    }
                }
                StopReason::SignalStop(pid, sign) => {
                    if !self.debugee.is_in_progress() {
                        continue;
                    }

                    self.expl_ctx_switch_thread(pid)?;
                    self.hooks.on_signal(sign);
                    break event;
                }
            }
        };

        Ok(stop_reason)
    }

    /// Restart debugee by recreating debugee process, save all user defined breakpoints.
    /// Return when new debugee stopped or ends.
    ///
    /// **! change exploration context**
    pub fn restart_debugee(&mut self) -> Result<Pid, Error> {
        match self.debugee.execution_status() {
            ExecutionStatus::Unload => {
                // all breakpoints already disabled by default
            }
            ExecutionStatus::InProgress => {
                print_warns!(self.breakpoints.disable_all_breakpoints(&self.debugee)?);
            }
            ExecutionStatus::Exited => {
                // all breakpoints already disabled by [`StopReason::DebugeeExit`] handler
            }
        }

        if !self.debugee.is_exited() {
            let proc_pid = self.process.pid();
            signal::kill(proc_pid, SIGKILL).map_err(|e| Syscall("kill", e))?;
            _ = self.debugee.tracer_mut().resume(TraceContext::new(&[]));
        }

        self.process = self.process.install()?;

        let new_debugee = self.debugee.extend(self.process.pid());
        _ = mem::replace(&mut self.debugee, new_debugee);

        // breakpoints will be enabled later, when StopReason::DebugeeStart state is reached
        self.breakpoints.update_pid(self.process.pid());

        self.hooks.on_process_install(self.process.pid());
        self.expl_context = ExplorationContext::new_non_running(self.process.pid());
        self.continue_execution()?;
        Ok(self.process.pid())
    }

    /// Start debugee.
    /// Return when debugee stopped or ends.
    pub fn start_debugee(&mut self) -> Result<(), Error> {
        if !self.debugee.is_in_progress() {
            self.continue_execution()?;
        }
        Ok(())
    }

    /// Continue debugee execution.
    pub fn continue_debugee(&mut self) -> Result<(), Error> {
        disable_when_not_stared!(self);
        self.continue_execution()?;
        Ok(())
    }

    /// Return list of symbols matching regular expression.
    ///
    /// # Arguments
    ///
    /// * `regex`: regular expression
    pub fn get_symbols(&self, regex: &str) -> Result<Vec<&Symbol>, Error> {
        let regex = Regex::new(regex)?;

        Ok(self
            .debugee
            .debug_info_all()
            .iter()
            .flat_map(|dwarf| dwarf.find_symbols(&regex))
            .collect())
    }

    /// Return in focus frame information.
    pub fn frame_info(&self) -> Result<FrameInfo, Error> {
        disable_when_not_stared!(self);
        self.debugee.frame_info(self.exploration_ctx())
    }

    /// Set new frame into focus.
    ///
    /// # Arguments
    ///
    /// * `num`: frame number in backtrace
    pub fn set_frame_into_focus(&mut self, num: u32) -> Result<u32, Error> {
        disable_when_not_stared!(self);
        let ctx = self.exploration_ctx();
        let backtrace = self.debugee.unwind(ctx.pid_on_focus())?;
        let frame = backtrace.get(num as usize).ok_or(FrameNotFound(num))?;
        self.expl_context = ExplorationContext::new(
            Location {
                pc: frame.ip,
                global_pc: frame.ip.into_global(&self.debugee)?,
                pid: ctx.pid_on_focus(),
            },
            num,
        );
        Ok(num)
    }

    /// Execute `on_step` callback with current exploration context
    fn execute_on_step_hook(&self) -> Result<(), Error> {
        let ctx = self.exploration_ctx();
        let pc = ctx.location().pc;
        let global_pc = ctx.location().global_pc;
        let dwarf = self.debugee.debug_info(pc)?;
        let place = weak_error!(dwarf.find_place_from_pc(global_pc)).flatten();
        let func = weak_error!(dwarf.find_function_by_pc(global_pc))
            .flatten()
            .map(|f| f.die);
        self.hooks.on_step(pc, place, func).map_err(Hook)
    }

    /// Do single step (until debugee reaches a different source line).
    ///
    /// **! change exploration context**
    pub fn step_into(&mut self) -> Result<(), Error> {
        disable_when_not_stared!(self);
        self.expl_ctx_restore_frame()?;

        match self.step_in()? {
            StepResult::Done => self.execute_on_step_hook(),
            StepResult::SignalInterrupt { signal, quiet } => {
                if !quiet {
                    self.hooks.on_signal(signal);
                }
                Ok(())
            }
        }
    }

    /// Move in focus thread to next instruction.
    ///
    /// **! change exploration context**
    pub fn stepi(&mut self) -> Result<(), Error> {
        disable_when_not_stared!(self);
        self.expl_ctx_restore_frame()?;

        if let Some(StopReason::SignalStop(_, sign)) = self.single_step_instruction()? {
            self.hooks.on_signal(sign);
            return Ok(());
        }

        self.execute_on_step_hook()
    }

    /// Return list of currently running debugee threads.
    pub fn thread_state(&self) -> Result<Vec<ThreadSnapshot>, Error> {
        disable_when_not_stared!(self);
        self.debugee.thread_state(self.exploration_ctx())
    }

    /// Sets the thread into focus.
    ///
    /// # Arguments
    ///
    /// * `num`: thread number
    pub fn set_thread_into_focus(&mut self, num: u32) -> Result<Tracee, Error> {
        disable_when_not_stared!(self);
        let tracee = self.debugee.get_tracee_by_num(num)?;
        self.expl_ctx_switch_thread(tracee.pid)?;
        Ok(tracee)
    }

    /// Return stack trace.
    ///
    /// # Arguments
    ///
    /// * `pid`: thread id
    pub fn backtrace(&self, pid: Pid) -> Result<Backtrace, Error> {
        disable_when_not_stared!(self);
        self.debugee.unwind(pid)
    }

    /// Read N bytes from debugee process.
    ///
    /// # Arguments
    ///
    /// * `addr`: address in debugee address space where reads
    /// * `read_n`: read byte count
    pub fn read_memory(&self, addr: usize, read_n: usize) -> Result<Vec<u8>, Error> {
        disable_when_not_stared!(self);
        read_memory_by_pid(self.debugee.tracee_ctl().proc_pid(), addr, read_n).map_err(Ptrace)
    }

    /// Write sizeof(uintptr_t) bytes in debugee address space
    ///
    /// # Arguments
    ///
    /// * `addr`: address to write
    /// * `value`: value to write
    pub fn write_memory(&self, addr: uintptr_t, value: uintptr_t) -> Result<(), Error> {
        disable_when_not_stared!(self);
        unsafe {
            sys::ptrace::write(
                self.debugee.tracee_ctl().proc_pid(),
                addr as *mut c_void,
                value as *mut c_void,
            )
            .map_err(Ptrace)
        }
    }

    /// Move to higher stack frame.
    pub fn step_out(&mut self) -> Result<(), Error> {
        disable_when_not_stared!(self);
        self.expl_ctx_restore_frame()?;
        self.step_out_frame()?;
        self.execute_on_step_hook()
    }

    /// Do debugee step (over subroutine calls to).
    pub fn step_over(&mut self) -> Result<(), Error> {
        disable_when_not_stared!(self);
        self.expl_ctx_restore_frame()?;
        match self.step_over_any()? {
            StepResult::Done => self.execute_on_step_hook(),
            StepResult::SignalInterrupt { signal, quiet } => {
                if !quiet {
                    self.hooks.on_signal(signal);
                }
                Ok(())
            }
        }
    }

    /// Reads all local variables from current function in current thread.
    pub fn read_local_variables(&self) -> Result<Vec<VariableIR>, Error> {
        disable_when_not_stared!(self);

        let evaluator = variable::select::SelectExpressionEvaluator::new(
            self,
            Expression::Variable(VariableSelector::Any),
        );
        evaluator.evaluate()
    }

    /// Reads any variable from the current thread, uses a select expression to filter variables
    /// and fetch their properties (such as structure fields or array elements).
    ///
    /// # Arguments
    ///
    /// * `select_expr`: data query expression
    pub fn read_variable(&self, select_expr: Expression) -> Result<Vec<VariableIR>, Error> {
        disable_when_not_stared!(self);
        let evaluator = variable::select::SelectExpressionEvaluator::new(self, select_expr);
        evaluator.evaluate()
    }

    ///  Reads any variable from the current thread, uses a select expression to filter variables
    /// and return their names.
    ///
    /// # Arguments
    ///
    /// * `select_expr`: data query expression
    pub fn read_variable_names(&self, select_expr: Expression) -> Result<Vec<String>, Error> {
        disable_when_not_stared!(self);
        let evaluator = variable::select::SelectExpressionEvaluator::new(self, select_expr);
        evaluator.evaluate_names()
    }

    /// Reads any argument from the current function, uses a select expression to filter variables
    /// and fetch their properties (such as structure fields or array elements).
    ///
    /// # Arguments
    ///
    /// * `select_expr`: data query expression
    pub fn read_argument(&self, select_expr: Expression) -> Result<Vec<VariableIR>, Error> {
        disable_when_not_stared!(self);
        let evaluator = variable::select::SelectExpressionEvaluator::new(self, select_expr);
        evaluator.evaluate_on_arguments()
    }

    /// Reads any argument from the current function, uses a select expression to filter arguments
    /// and return their names.
    ///
    /// # Arguments
    ///
    /// * `select_expr`: data query expression
    pub fn read_argument_names(&self, select_expr: Expression) -> Result<Vec<String>, Error> {
        disable_when_not_stared!(self);
        let evaluator = variable::select::SelectExpressionEvaluator::new(self, select_expr);
        evaluator.evaluate_on_arguments_names()
    }

    /// Return following register value.
    ///
    /// # Arguments
    ///
    /// * `register_name`: x86-64 register name (ex: `rip`)
    pub fn get_register_value(&self, register_name: &str) -> Result<u64, Error> {
        disable_when_not_stared!(self);

        let r = Register::from_str(register_name)
            .map_err(|_| RegisterNameNotFound(register_name.into()))?;
        Ok(RegisterMap::current(self.exploration_ctx().pid_on_focus())?.value(r))
    }

    /// Return registers dump for on focus thread at instruction defined by pc.
    ///
    /// # Arguments
    ///
    /// * `pc`: program counter value
    pub fn current_thread_registers_at_pc(
        &self,
        pc: RelocatedAddress,
    ) -> Result<DwarfRegisterMap, Error> {
        disable_when_not_stared!(self);
        let unwinder = DwarfUnwinder::new(&self.debugee);
        let location = Location {
            pc,
            global_pc: pc.into_global(&self.debugee)?,
            pid: self.exploration_ctx().pid_on_focus(),
        };
        Ok(unwinder
            // there is no chance to determine frame number, cause pc may owned by code outside backtrace
            // so set frame num to 0 is ok
            .context_for(&ExplorationContext::new(location, 0))?
            .ok_or(UnwindNoContext)?
            .registers())
    }

    /// Set new register value.
    ///
    /// # Arguments
    ///
    /// * `register_name`: x86-64 register name (ex: `rip`)
    /// * `val`: 8 bite value
    pub fn set_register_value(&self, register_name: &str, val: u64) -> Result<(), Error> {
        disable_when_not_stared!(self);

        let in_focus_pid = self.exploration_ctx().pid_on_focus();
        let mut map = RegisterMap::current(in_focus_pid)?;
        map.update(
            Register::try_from(register_name)
                .map_err(|_| RegisterNameNotFound(register_name.into()))?,
            val,
        );
        map.persist(in_focus_pid)
    }

    /// Return list of known files income from dwarf parser.
    pub fn known_files(&self) -> impl Iterator<Item = &PathBuf> {
        self.debugee
            .debug_info_all()
            .into_iter()
            .filter_map(|dwarf| dwarf.known_files().ok())
            .flatten()
    }

    /// Return a list of shared libraries.
    pub fn shared_libs(&self) -> Vec<RegionInfo> {
        self.debugee.dump_mapped_regions()
    }

    /// Return a list of disassembled instruction for a function in focus.
    pub fn disasm(&self) -> Result<FunctionAssembly, Error> {
        self.debugee.disasm(
            self.exploration_ctx(),
            &self.breakpoints.active_breakpoints(),
        )
    }
}

impl Drop for Debugger {
    fn drop(&mut self) {
        match self.debugee.execution_status() {
            ExecutionStatus::Unload => {
                signal::kill(self.debugee.tracee_ctl().proc_pid(), Signal::SIGKILL)
                    .expect("kill debugee");
                waitpid(self.debugee.tracee_ctl().proc_pid(), None).expect("waiting child");
            }
            ExecutionStatus::InProgress => {
                // ignore all possible errors on breakpoints disabling
                _ = self.breakpoints.disable_all_breakpoints(&self.debugee);

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
                let wait_result = loop {
                    let wait_result = waitpid(Pid::from_raw(-1), None).expect("waiting debugee");
                    if wait_result.pid() == Some(self.debugee.tracee_ctl().proc_pid()) {
                        break wait_result;
                    }
                };

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
pub fn read_memory_by_pid(pid: Pid, addr: usize, read_n: usize) -> Result<Vec<u8>, nix::Error> {
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
