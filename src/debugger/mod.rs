pub mod address;
mod breakpoint;
mod code;
pub mod command;
mod debugee;
pub mod process;
pub mod register;
pub mod rust;
mod step;
mod utils;
pub mod variable;

pub use breakpoint::BreakpointView;
pub use debugee::dwarf::r#type::TypeDeclaration;
pub use debugee::dwarf::unit::FunctionDie;
pub use debugee::dwarf::unit::PlaceDescriptor;
pub use debugee::dwarf::unwind;
pub use debugee::ThreadSnapshot;

use crate::debugger::address::{Address, GlobalAddress, RelocatedAddress};
use crate::debugger::breakpoint::{Breakpoint, BreakpointRegistry, BrkptType, UninitBreakpoint};
use crate::debugger::debugee::dwarf::r#type::TypeCache;
use crate::debugger::debugee::dwarf::unwind::Backtrace;
use crate::debugger::debugee::dwarf::{DwarfUnwinder, Symbol};
use crate::debugger::debugee::tracee::Tracee;
use crate::debugger::debugee::tracer::{StopReason, TraceContext};
use crate::debugger::debugee::{Debugee, ExecutionStatus, FrameInfo, Location};
use crate::debugger::process::{Child, Installed};
use crate::debugger::register::{DwarfRegisterMap, Register, RegisterMap};
use crate::debugger::variable::select::{Expression, VariableSelector};
use crate::debugger::variable::VariableIR;
use crate::weak_error;
use anyhow::anyhow;
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

macro_rules! disable_when_not_stared {
    ($this: expr) => {
        use anyhow::bail;
        if $this.debugee.execution_status != ExecutionStatus::InProgress {
            bail!("The program is not being started.")
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

    /// Return last set exploration context.
    #[inline(always)]
    pub fn exploration_ctx(&self) -> &ExplorationContext {
        &self.expl_context
    }

    /// Update current program counters for current in focus thread.
    fn expl_ctx_update_location(&mut self) -> anyhow::Result<&ExplorationContext> {
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
    fn expl_ctx_restore_frame(&mut self) -> anyhow::Result<&ExplorationContext> {
        self.expl_ctx_update_location()
    }

    /// Change in focus thread and update program counters.
    ///
    /// # Arguments
    ///
    /// * `pid`: new in focus thread id
    fn expl_ctx_switch_thread(&mut self, pid: Pid) -> anyhow::Result<&ExplorationContext> {
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
    fn continue_execution(&mut self) -> anyhow::Result<()> {
        self.step_over_breakpoint()?;

        loop {
            let event = self
                .debugee
                .trace_until_stop(TraceContext::new(&self.breakpoints.active_breakpoints()))?;
            match event {
                StopReason::DebugeeExit(code) => {
                    self.hooks.on_exit(code);
                    break;
                }
                StopReason::DebugeeStart => {
                    self.breakpoints.enable_all_breakpoints(&self.debugee)?;
                    //self.expl_ctx_update_location()?;
                }
                StopReason::NoSuchProcess(_) => {
                    break;
                }
                StopReason::Breakpoint(pid, current_pc) => {
                    self.expl_ctx_switch_thread(pid)?;

                    if let Some(bp) = self.breakpoints.get_enabled(current_pc) {
                        match bp.r#type() {
                            BrkptType::EntryPoint => {
                                self.step_over_breakpoint()?;
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
                                    .on_breakpoint(current_pc, bp.number(), place, func)?;
                                break;
                            }
                            BrkptType::Temporary => {
                                break;
                            }
                        }
                    }
                }
                StopReason::SignalStop(pid, sign) => {
                    self.expl_ctx_switch_thread(pid)?;
                    self.hooks.on_signal(sign);
                    break;
                }
            }
        }

        Ok(())
    }

    /// Restart debugee by recreating debugee process, save all user defined breakpoints.
    /// Return when new debugee stopped or ends.
    ///
    /// **! change exploration context**
    pub fn restart_debugee(&mut self) -> anyhow::Result<()> {
        if self.debugee.execution_status == ExecutionStatus::InProgress {
            self.breakpoints.disable_all_breakpoints(&self.debugee)?;
        }

        if self.debugee.execution_status != ExecutionStatus::Exited {
            let proc_pid = self.debugee.tracee_ctl().proc_pid();
            signal::kill(proc_pid, SIGKILL)?;
            _ = self.debugee.tracer_mut().resume(TraceContext::new(&vec![]));
        }

        self.process = self.process.install()?;

        let new_debugee = self.debugee.extend(self.process.pid());
        _ = mem::replace(&mut self.debugee, new_debugee);

        // breakpoints will be enabled later, when StopReason::DebugeeStart state is reached
        self.breakpoints.update_pid(self.process.pid());

        self.hooks.on_process_install(self.process.pid());
        self.expl_context = ExplorationContext::new_non_running(self.process.pid());
        self.continue_execution()
    }

    /// Start debugee.
    /// Return when debugee stopped or ends.
    pub fn start_debugee(&mut self) -> anyhow::Result<()> {
        if self.debugee.execution_status != ExecutionStatus::InProgress {
            return self.continue_execution();
        }
        Ok(())
    }

    /// Continue debugee execution.
    pub fn continue_debugee(&mut self) -> anyhow::Result<()> {
        disable_when_not_stared!(self);
        self.continue_execution()
    }

    /// Return list of symbols matching regular expression.
    ///
    /// # Arguments
    ///
    /// * `regex`: regular expression
    pub fn get_symbols(&self, regex: &str) -> anyhow::Result<Vec<&Symbol>> {
        let regex = Regex::new(regex)?;

        Ok(self
            .debugee
            .debug_info_all()
            .iter()
            .flat_map(|dwarf| dwarf.find_symbols(&regex))
            .collect())
    }

    /// Return in focus frame information.
    pub fn frame_info(&self) -> anyhow::Result<FrameInfo> {
        disable_when_not_stared!(self);
        self.debugee.frame_info(self.exploration_ctx())
    }

    /// Set new frame into focus.
    ///
    /// # Arguments
    ///
    /// * `num`: frame number in backtrace
    pub fn set_frame_into_focus(&mut self, num: u32) -> anyhow::Result<u32> {
        disable_when_not_stared!(self);
        let ctx = self.exploration_ctx();
        let backtrace = self.debugee.unwind(ctx.pid_on_focus())?;
        let frame = backtrace
            .get(num as usize)
            .ok_or(anyhow!("frame {num} not found"))?;
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
    fn execute_on_step_hook(&self) -> anyhow::Result<()> {
        let ctx = self.exploration_ctx();
        let pc = ctx.location().pc;
        let global_pc = ctx.location().global_pc;
        let dwarf = self.debugee.debug_info(pc)?;
        let place = weak_error!(dwarf.find_place_from_pc(global_pc)).flatten();
        let func = weak_error!(dwarf.find_function_by_pc(global_pc))
            .flatten()
            .map(|f| f.die);
        self.hooks.on_step(pc, place, func)
    }

    /// Do single step (until debugee reaches a different source line).
    ///
    /// **! change exploration context**
    pub fn step_into(&mut self) -> anyhow::Result<()> {
        disable_when_not_stared!(self);
        self.expl_ctx_restore_frame()?;
        self.step_in()?;
        self.execute_on_step_hook()
    }

    /// Move in focus thread to next instruction.
    ///
    /// **! change exploration context**
    pub fn stepi(&mut self) -> anyhow::Result<()> {
        disable_when_not_stared!(self);
        self.expl_ctx_restore_frame()?;

        self.single_step_instruction()?;
        self.execute_on_step_hook()
    }

    /// Return list of currently running debugee threads.
    pub fn thread_state(&self) -> anyhow::Result<Vec<ThreadSnapshot>> {
        disable_when_not_stared!(self);
        self.debugee.thread_state(self.exploration_ctx())
    }

    /// Sets the thread into focus.
    ///
    /// # Arguments
    ///
    /// * `num`: thread number
    pub fn set_thread_into_focus(&mut self, num: u32) -> anyhow::Result<Tracee> {
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
    pub fn backtrace(&self, pid: Pid) -> anyhow::Result<Backtrace> {
        disable_when_not_stared!(self);
        self.debugee.unwind(pid)
    }

    /// Read N bytes from debugee process.
    ///
    /// # Arguments
    ///
    /// * `addr`: address in debugee address space where reads
    /// * `read_n`: read byte count
    pub fn read_memory(&self, addr: usize, read_n: usize) -> anyhow::Result<Vec<u8>> {
        disable_when_not_stared!(self);
        Ok(read_memory_by_pid(
            self.debugee.tracee_ctl().proc_pid(),
            addr,
            read_n,
        )?)
    }

    /// Write sizeof(uintptr_t) bytes in debugee address space
    ///
    /// # Arguments
    ///
    /// * `addr`: address to write
    /// * `value`: value to write
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

    /// Move to higher stack frame.
    pub fn step_out(&mut self) -> anyhow::Result<()> {
        disable_when_not_stared!(self);
        self.expl_ctx_restore_frame()?;
        self.step_out_frame()?;
        self.execute_on_step_hook()
    }

    /// Do debugee step (over subroutine calls to).
    pub fn step_over(&mut self) -> anyhow::Result<()> {
        disable_when_not_stared!(self);
        self.expl_ctx_restore_frame()?;
        self.step_over_any()?;
        self.execute_on_step_hook()
    }

    /// Create and enable breakpoint at debugee address space
    ///
    /// # Arguments
    ///
    /// * `addr`: address where debugee must be stopped
    pub fn set_breakpoint_at_addr(
        &mut self,
        addr: RelocatedAddress,
    ) -> anyhow::Result<BreakpointView> {
        if self.debugee.in_progress() {
            let dwarf = self.debugee.debug_info(addr);
            let global_addr = addr.into_global(&self.debugee)?;

            let place = dwarf
                .ok()
                .map(|dwarf| {
                    dwarf
                        .find_place_from_pc(global_addr)?
                        .map(|p| p.to_owned())
                        .ok_or(anyhow!("unknown address"))
                })
                .transpose()?;

            self.breakpoints
                .add_and_enable(Breakpoint::new(addr, self.process.pid(), place))
        } else {
            Ok(self.breakpoints.add_uninit(UninitBreakpoint::new(
                Address::Relocated(addr),
                self.process.pid(),
                None,
            )))
        }
    }

    /// Disable and remove breakpoint by its address.
    ///
    /// # Arguments
    ///
    /// * `addr`: breakpoint address
    fn remove_breakpoint(&mut self, addr: Address) -> anyhow::Result<Option<BreakpointView>> {
        self.breakpoints.remove_by_addr(addr)
    }

    /// Disable and remove breakpoint by its address.
    ///
    /// # Arguments
    ///
    /// * `addr`: breakpoint address
    pub fn remove_breakpoint_at_addr(
        &mut self,
        addr: RelocatedAddress,
    ) -> anyhow::Result<Option<BreakpointView>> {
        self.breakpoints.remove_by_addr(Address::Relocated(addr))
    }

    /// Create and enable breakpoint at debugee address space on the following function start.
    ///
    /// # Arguments
    ///
    /// * `name`: function name where debugee must be stopped
    pub fn set_breakpoint_at_fn(&mut self, name: &str) -> anyhow::Result<BreakpointView> {
        // todo: currently you can set breakpoint only at functions that belongs to executable object
        let dwarf = self.debugee.program_debug_info()?;
        let place = dwarf.get_function_place(name)?;

        if self.debugee.in_progress() {
            let addr = place
                .address
                .relocate(self.debugee.mapping_offset_for_file(dwarf)?);
            self.breakpoints
                .add_and_enable(Breakpoint::new(addr, self.process.pid(), Some(place)))
        } else {
            Ok(self.breakpoints.add_uninit(UninitBreakpoint::new(
                Address::Global(place.address),
                self.process.pid(),
                Some(place),
            )))
        }
    }

    /// Disable and remove breakpoint from function start.
    ///
    /// # Arguments
    ///
    /// * `name`: function name
    pub fn remove_breakpoint_at_fn(
        &mut self,
        name: &str,
    ) -> anyhow::Result<Option<BreakpointView>> {
        // todo: currently you can set breakpoint only at addresses that belongs to executable object
        let dwarf = self.debugee.program_debug_info()?;
        let place = dwarf.get_function_place(name)?;
        if self.debugee.in_progress() {
            self.breakpoints.remove_by_addr(Address::Relocated(
                place
                    .address
                    .relocate(self.debugee.mapping_offset_for_file(dwarf)?),
            ))
        } else {
            self.breakpoints
                .remove_by_addr(Address::Global(place.address))
        }
    }

    /// Create and enable breakpoint at the following file and line number.
    ///
    /// # Arguments
    ///
    /// * `fine_name`: file name (ex: "main.rs")
    /// * `line`: line number
    pub fn set_breakpoint_at_line(
        &mut self,
        fine_name: &str,
        line: u64,
    ) -> anyhow::Result<BreakpointView> {
        // todo: currently you can set breakpoint only at addresses that belongs to executable object
        let dwarf = self.debugee.program_debug_info()?;
        let place = dwarf
            .find_place(fine_name, line)?
            .ok_or(anyhow!("no source file/line"))?
            .to_owned();

        if self.debugee.in_progress() {
            let addr = place
                .address
                .relocate(self.debugee.mapping_offset_for_file(dwarf)?);
            self.breakpoints
                .add_and_enable(Breakpoint::new(addr, self.process.pid(), Some(place)))
        } else {
            Ok(self.breakpoints.add_uninit(UninitBreakpoint::new(
                Address::Global(place.address),
                self.process.pid(),
                Some(place),
            )))
        }
    }

    /// Disable and remove breakpoint at the following file and line number.
    ///
    /// # Arguments
    ///
    /// * `fine_name`: file name (ex: "main.rs")
    /// * `line`: line number
    pub fn remove_breakpoint_at_line(
        &mut self,
        fine_name: &str,
        line: u64,
    ) -> anyhow::Result<Option<BreakpointView>> {
        // todo: currently you can set breakpoint only at addresses that belongs to executable object
        let dwarf = self.debugee.program_debug_info()?;
        let place = dwarf
            .find_place(fine_name, line)?
            .ok_or(anyhow!("no source file/line"))?;

        if self.debugee.in_progress() {
            self.breakpoints.remove_by_addr(Address::Relocated(
                place
                    .address
                    .relocate(self.debugee.mapping_offset_for_file(dwarf)?),
            ))
        } else {
            self.breakpoints
                .remove_by_addr(Address::Global(place.address))
        }
    }

    /// Return list of breakpoints.
    pub fn breakpoints_snapshot(&self) -> Vec<BreakpointView> {
        self.breakpoints.snapshot()
    }

    /// Reads all local variables from current function in current thread.
    pub fn read_local_variables(&self) -> anyhow::Result<Vec<VariableIR>> {
        disable_when_not_stared!(self);

        let evaluator = variable::select::SelectExpressionEvaluator::new(
            self,
            Expression::Variable(VariableSelector::Any),
        )?;
        evaluator.evaluate()
    }

    /// Reads any variable from the current thread, uses a select expression to filter variables
    /// and fetch their properties (such as structure fields or array elements).
    ///
    /// # Arguments
    ///
    /// * `select_expr`: data query expression
    pub fn read_variable(&self, select_expr: Expression) -> anyhow::Result<Vec<VariableIR>> {
        disable_when_not_stared!(self);
        let evaluator = variable::select::SelectExpressionEvaluator::new(self, select_expr)?;
        evaluator.evaluate()
    }

    ///  Reads any variable from the current thread, uses a select expression to filter variables
    /// and return their names.
    ///
    /// # Arguments
    ///
    /// * `select_expr`: data query expression
    pub fn read_variable_names(&self, select_expr: Expression) -> anyhow::Result<Vec<String>> {
        disable_when_not_stared!(self);
        let evaluator = variable::select::SelectExpressionEvaluator::new(self, select_expr)?;
        evaluator.evaluate_names()
    }

    /// Reads any argument from the current function, uses a select expression to filter variables
    /// and fetch their properties (such as structure fields or array elements).
    ///
    /// # Arguments
    ///
    /// * `select_expr`: data query expression
    pub fn read_argument(&self, select_expr: Expression) -> anyhow::Result<Vec<VariableIR>> {
        disable_when_not_stared!(self);
        let evaluator = variable::select::SelectExpressionEvaluator::new(self, select_expr)?;
        evaluator.evaluate_on_arguments()
    }

    /// Reads any argument from the current function, uses a select expression to filter arguments
    /// and return their names.
    ///
    /// # Arguments
    ///
    /// * `select_expr`: data query expression
    pub fn read_argument_names(&self, select_expr: Expression) -> anyhow::Result<Vec<String>> {
        disable_when_not_stared!(self);
        let evaluator = variable::select::SelectExpressionEvaluator::new(self, select_expr)?;
        evaluator.evaluate_on_arguments_names()
    }

    /// Return following register value.
    ///
    /// # Arguments
    ///
    /// * `register_name`: x86-64 register name (ex: `rip`)
    pub fn get_register_value(&self, register_name: &str) -> anyhow::Result<u64> {
        disable_when_not_stared!(self);

        Ok(RegisterMap::current(self.exploration_ctx().pid_on_focus())?
            .value(Register::from_str(register_name)?))
    }

    /// Return registers dump for on focus thread at instruction defined by pc.
    ///
    /// # Arguments
    ///
    /// * `pc`: program counter value
    pub fn current_thread_registers_at_pc(
        &self,
        pc: RelocatedAddress,
    ) -> anyhow::Result<DwarfRegisterMap> {
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
            .ok_or(anyhow!("fetch register fail"))?
            .registers())
    }

    /// Set new register value.
    ///
    /// # Arguments
    ///
    /// * `register_name`: x86-64 register name (ex: `rip`)
    /// * `val`: 8 bite value
    pub fn set_register_value(&self, register_name: &str, val: u64) -> anyhow::Result<()> {
        disable_when_not_stared!(self);

        let in_focus_pid = self.exploration_ctx().pid_on_focus();
        let mut map = RegisterMap::current(in_focus_pid)?;
        map.update(Register::try_from(register_name)?, val);
        Ok(map.persist(in_focus_pid)?)
    }

    /// Return list of known files income from dwarf parser.
    pub fn known_files(&self) -> impl Iterator<Item = &PathBuf> {
        self.debugee
            .debug_info_all()
            .into_iter()
            .filter_map(|dwarf| dwarf.known_files().ok())
            .flatten()
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
