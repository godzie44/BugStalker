pub mod address;
pub mod r#async;
mod breakpoint;
pub mod call;
mod code;
mod context;
mod debugee;
mod error;
pub mod process;
pub mod register;
pub mod rust;
mod step;
mod utils;
pub mod variable;
mod watchpoint;

pub use breakpoint::BreakpointView;
pub use breakpoint::BreakpointViewOwned;
pub use breakpoint::CreateTransparentBreakpointRequest;
pub use debugee::FrameInfo;
pub use debugee::FunctionAssembly;
pub use debugee::FunctionRange;
pub use debugee::RegionInfo;
pub use debugee::ThreadSnapshot;
pub use debugee::dwarf::Symbol;
pub use debugee::dwarf::r#type::ComplexType;
pub use debugee::dwarf::r#type::TypeDeclaration;
pub use debugee::dwarf::unit::FunctionInfo;
pub use debugee::dwarf::unit::PlaceDescriptor;
pub use debugee::dwarf::unit::PlaceDescriptorOwned;
/// Public unwind API backed by the internal DWARF unwinder (no libunwind feature gate).
pub use debugee::dwarf::unwind;
pub use debugee::tracee::Tracee;
pub use debugee::tracer::StopReason;
pub use error::Error;
pub use watchpoint::WatchpointView;
pub use watchpoint::WatchpointViewOwned;

use crate::debugger::Error::Syscall;
use crate::debugger::address::{Address, GlobalAddress, RelocatedAddress};
use crate::debugger::breakpoint::{Breakpoint, BreakpointRegistry, BrkptType, UninitBreakpoint};
use crate::debugger::debugee::dwarf::DwarfUnwinder;
use crate::debugger::debugee::dwarf::unwind::Backtrace;
use crate::debugger::debugee::tracer::TraceContext;
use crate::debugger::debugee::{Debugee, ExecutionStatus, Location};
use crate::debugger::error::Error::{
    FrameNotFound, Hook, ProcessNotStarted, Ptrace, RegisterNameNotFound, UnwindNoContext,
};
use crate::debugger::process::{Child, Installed};
use crate::debugger::register::debug::BreakCondition;
use crate::debugger::register::{DwarfRegisterMap, Register, RegisterMap};
use crate::debugger::step::StepResult;
use crate::debugger::variable::dqe::{Dqe, Selector};
use crate::debugger::variable::execute::QueryResult;
use crate::debugger::variable::value::Value;
use crate::debugger::watchpoint::WatchpointRegistry;
use crate::oracle::Oracle;
use crate::{print_warns, weak_error};
use indexmap::IndexMap;
use log::debug;
use nix::libc::{c_void, uintptr_t};
use nix::sys;
use nix::sys::signal;
use nix::sys::signal::{SIGKILL, Signal};
use nix::sys::wait::{WaitStatus, waitpid};
use nix::unistd::Pid;
use object::Object;
use os_pipe::PipeWriter;
use regex::Regex;
use std::ffi::c_long;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::{fs, mem};

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
    /// * `thread_num`: number of in focus thread
    fn on_breakpoint(
        &self,
        pc: RelocatedAddress,
        num: u32,
        place: Option<PlaceDescriptor>,
        function: Option<&FunctionInfo>,
        thread_num: Option<u32>,
    ) -> anyhow::Result<()>;

    /// Called when watchpoint is activated.
    ///
    /// # Arguments
    ///
    /// * `pc`: address of instruction where breakpoint is reached
    /// * `num`: breakpoint number
    /// * `place`: breakpoint number
    /// * `condition`: reason of a watchpoint activation
    /// * `dqe_string`: stringified data query expression (if exist)
    /// * `old_value`: previous expression or mem location value
    /// * `new_value`: current expression or mem location value
    /// * `end_of_scope`: true if watchpoint activated cause end of scope is reached
    #[allow(clippy::too_many_arguments)]
    fn on_watchpoint(
        &self,
        pc: RelocatedAddress,
        num: u32,
        place: Option<PlaceDescriptor>,
        condition: BreakCondition,
        dqe_string: Option<&str>,
        old_value: Option<&Value>,
        new_value: Option<&Value>,
        end_of_scope: bool,
    ) -> anyhow::Result<()>;

    /// Called when one of step commands is done.
    ///
    /// # Arguments
    ///
    /// * `pc`: address of instruction where breakpoint is reached
    /// * `place`: stop place information
    /// * `function`: function debug information entry
    /// * `thread_num`: number of in focus thread
    fn on_step(
        &self,
        pc: RelocatedAddress,
        place: Option<PlaceDescriptor>,
        function: Option<&FunctionInfo>,
        thread_num: Option<u32>,
    ) -> anyhow::Result<()>;

    /// Called when one of async step commands is done.
    ///
    /// # Arguments
    ///
    /// * `pc`: address of instruction where breakpoint is reached
    /// * `place`: stop place information
    /// * `function`: function debug information entry
    /// * `task_id`: asynchronous task id
    /// * `task_completed`: true if task is already completed
    fn on_async_step(
        &self,
        pc: RelocatedAddress,
        place: Option<PlaceDescriptor>,
        function: Option<&FunctionInfo>,
        task_id: u64,
        task_completed: bool,
    ) -> anyhow::Result<()>;

    /// Called when debugee receive an OS signal. Debugee is in signal-stop at this moment.
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
    fn on_process_install(&self, pid: Pid, object: Option<&object::File>);
}

pub struct NopHook {}

impl EventHook for NopHook {
    fn on_breakpoint(
        &self,
        _: RelocatedAddress,
        _: u32,
        _: Option<PlaceDescriptor>,
        _: Option<&FunctionInfo>,
        _: Option<u32>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn on_watchpoint(
        &self,
        _: RelocatedAddress,
        _: u32,
        _: Option<PlaceDescriptor>,
        _: BreakCondition,
        _: Option<&str>,
        _: Option<&Value>,
        _: Option<&Value>,
        _: bool,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn on_step(
        &self,
        _: RelocatedAddress,
        _: Option<PlaceDescriptor>,
        _: Option<&FunctionInfo>,
        _: Option<u32>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn on_async_step(
        &self,
        _: RelocatedAddress,
        _: Option<PlaceDescriptor>,
        _: Option<&FunctionInfo>,
        _: u64,
        _: bool,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn on_signal(&self, _: Signal) {}

    fn on_exit(&self, _: i32) {}

    fn on_process_install(&self, _: Pid, _: Option<&object::File>) {}
}

#[macro_export]
macro_rules! disable_when_not_stared {
    ($this: expr) => {
        if !$this.debugee.is_in_progress() {
            return Err($crate::debugger::error::Error::ProcessNotStarted);
        }
    };
}

/// Exploration context (or ecx). Contains current explored thread and program counter.
/// May be changed by user (by `thread` or `frame` command)
/// or by debugger (at breakpoints, after steps, etc.).
#[derive(Clone, Debug)]
pub struct ExplorationContext {
    focus_location: Location,
    focus_frame: u32,
}

impl ExplorationContext {
    /// Create a new context with known thread but without known program counter-value.
    /// It is useful when debugee is not started yet or restarted.
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
    pub fn frame_num(&self) -> u32 {
        self.focus_frame
    }

    #[inline(always)]
    pub fn pid_on_focus(&self) -> Pid {
        self.location().pid
    }
}

/// Debugger structure builder.
#[derive(Default)]
pub struct DebuggerBuilder<H: EventHook + 'static = NopHook> {
    oracles: Vec<Arc<dyn Oracle>>,
    hooks: Option<H>,
}

impl<H: EventHook + 'static> DebuggerBuilder<H> {
    /// Create a new builder.
    pub fn new() -> Self {
        Self {
            oracles: vec![],
            hooks: None,
        }
    }

    /// Add oracles.
    ///
    /// # Arguments
    ///
    /// * `oracles`: list of oracles
    pub fn with_oracles(self, oracles: Vec<Arc<dyn Oracle>>) -> Self {
        Self { oracles, ..self }
    }

    /// Add event hooks implementation
    ///
    /// # Arguments
    ///
    /// * `hooks`: hooks implementation
    pub fn with_hooks(self, hooks: H) -> Self {
        Self {
            hooks: Some(hooks),
            ..self
        }
    }

    /// Return all oracles.
    pub fn oracles(&self) -> impl Iterator<Item = &dyn Oracle> {
        self.oracles.iter().map(|oracle| oracle.as_ref())
    }

    /// Create a debugger.
    ///
    /// # Arguments
    ///
    /// * `process`: debugee process
    pub fn build(self, process: Child<Installed>) -> Result<Debugger, Error> {
        if let Some(hooks) = self.hooks {
            Debugger::new(process, hooks, self.oracles)
        } else {
            Debugger::new(process, NopHook {}, self.oracles)
        }
    }

    /// Create a debugger attached to a running process.
    ///
    /// # Arguments
    ///
    /// * `pid`: debugee process id
    /// * `stdout`: stdout pipe for future restarts
    /// * `stderr`: stderr pipe for future restarts
    pub fn build_attached(
        self,
        pid: Pid,
        stdout: PipeWriter,
        stderr: PipeWriter,
    ) -> Result<Debugger, Error> {
        let process = Child::from_external(pid, stdout, stderr)?;
        self.build(process)
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
    /// Watchpoints lists.
    watchpoints: WatchpointRegistry,
    /// Debugger interrupt with UI by EventHook trait.
    hooks: Box<dyn EventHook>,
    /// Current exploration context.
    expl_context: ExplorationContext,
    /// Map of name -> (oracle, installed flag) pairs.
    oracles: IndexMap<&'static str, (Arc<dyn Oracle>, bool)>,
    /// Detach flag to skip destructive cleanup on drop.
    detached: bool,
}

impl Debugger {
    fn new(
        process: Child<Installed>,
        hooks: impl EventHook + 'static,
        oracles: impl IntoIterator<Item = Arc<dyn Oracle>>,
    ) -> Result<Self, Error> {
        let program_path = Path::new(process.program());

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
        hooks.on_process_install(process_id, Some(&object));

        let debugee = if process.is_external() {
            Debugee::new_from_external_process(program_path, &process, &object)?
        } else {
            Debugee::new_non_running(program_path, &process, &object)?
        };

        Ok(Self {
            debugee,
            process,
            breakpoints,
            watchpoints: WatchpointRegistry::default(),
            hooks: Box::new(hooks),
            expl_context: ExplorationContext::new_non_running(process_id),
            oracles: oracles
                .into_iter()
                .map(|oracle| (oracle.name(), (oracle, false)))
                .collect(),
            detached: false,
        })
    }

    /// Return installed oracle, or `None` if oracle not found or not installed.
    ///
    /// # Arguments
    ///
    /// * `name`: oracle name
    pub fn get_oracle(&self, name: &str) -> Option<&dyn Oracle> {
        self.oracles
            .get(name)
            .and_then(|(oracle, install)| install.then_some(oracle.as_ref()))
    }

    /// Same as `get_oracle` but return an `Arc<dyn Oracle>`
    pub fn get_oracle_arc(&self, name: &str) -> Option<Arc<dyn Oracle>> {
        self.oracles
            .get(name)
            .and_then(|(oracle, install)| install.then_some(oracle.clone()))
    }

    /// Return all oracles.
    pub fn all_oracles(&self) -> impl Iterator<Item = &dyn Oracle> {
        self.oracles.values().map(|(oracle, _)| oracle.as_ref())
    }

    /// Same as `all_oracles` but return iterator over `Arc<dyn Oracle>`
    pub fn all_oracles_arc(&self) -> impl Iterator<Item = Arc<dyn Oracle>> + '_ {
        self.oracles.values().map(|(oracle, _)| oracle.clone())
    }

    pub fn process(&self) -> &Child<Installed> {
        &self.process
    }

    pub fn detach(&mut self) -> Result<(), Error> {
        if self.detached {
            return Ok(());
        }

        _ = self.breakpoints.disable_all_breakpoints(&self.debugee);
        self.watchpoints
            .clear_all(self.debugee.tracee_ctl(), &mut self.breakpoints);

        let current_tids: Vec<Pid> = self
            .debugee
            .tracee_ctl()
            .tracee_iter()
            .map(|t| t.pid)
            .collect();

        if !current_tids.is_empty() {
            current_tids
                .iter()
                .try_for_each(|tid| sys::ptrace::detach(*tid, None).map_err(Ptrace))?;

            signal::kill(self.debugee.tracee_ctl().proc_pid(), Signal::SIGCONT)
                .map_err(|e| Syscall("kill", e))?;
        }

        self.detached = true;
        Ok(())
    }

    pub fn set_hook(&mut self, hooks: impl EventHook + 'static) {
        self.hooks = Box::new(hooks);
    }

    /// Return last set exploration context.
    #[inline(always)]
    pub fn ecx(&self) -> &ExplorationContext {
        &self.expl_context
    }

    /// Update current program counters for current in focus thread.
    fn ecx_update_location(&mut self) -> Result<&ExplorationContext, Error> {
        let old_ecx = self.ecx();
        self.expl_context = ExplorationContext::new(
            self.debugee
                .get_tracee_ensure(old_ecx.pid_on_focus())
                .location(&self.debugee)?,
            0,
        );
        Ok(&self.expl_context)
    }

    fn ecx_swap(&mut self, new: ExplorationContext) {
        self.expl_context = new;
    }

    /// Restore frame from user defined to real.
    fn ecx_restore_frame(&mut self) -> Result<&ExplorationContext, Error> {
        self.ecx_update_location()
    }

    /// Change in focus thread and update program counters.
    ///
    /// # Arguments
    ///
    /// * `pid`: new in focus thread id
    fn ecx_switch_thread(&mut self, pid: Pid) -> Result<&ExplorationContext, Error> {
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
        if let Some(sign_or_wp) = self.step_over_breakpoint()? {
            match sign_or_wp {
                StopReason::Watchpoint(pid, current_pc, ty) => {
                    self.execute_on_watchpoint_hook(pid, current_pc, &ty)?;
                    return Ok(StopReason::Watchpoint(pid, current_pc, ty));
                }
                StopReason::SignalStop(pid, sign) => {
                    self.hooks.on_signal(sign);
                    return Ok(StopReason::SignalStop(pid, sign));
                }
                _ => {
                    unreachable!("unexpected reason")
                }
            }
        }

        let stop_reason = loop {
            let event = self.debugee.trace_until_stop(TraceContext::new(
                &self.breakpoints.active_breakpoints(),
                &self.watchpoints,
            ))?;
            match event {
                StopReason::DebugeeExit(code) => {
                    // ignore all possible errors on watchpoints disabling
                    _ = self.watchpoints.clear_local_disable_global(
                        self.debugee.tracee_ctl(),
                        &mut self.breakpoints,
                    );
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
                    self.ecx_switch_thread(pid)?;

                    if let Some(bp) = self.breakpoints.get_enabled(current_pc) {
                        match bp.r#type() {
                            BrkptType::EntryPoint => {
                                print_warns!(
                                    self.breakpoints.enable_all_breakpoints(&self.debugee)
                                );
                                print_warns!(self.watchpoints.refresh(&self.debugee));

                                // rendezvous already available at this point
                                let brk = self.debugee.rendezvous().r_brk();
                                self.breakpoints.add_and_enable(Breakpoint::new_linker_map(
                                    brk,
                                    self.process.pid(),
                                ))?;

                                // check oracles is ready
                                let oracles = self.oracles.clone();
                                self.oracles = oracles.into_iter().map(|(key, (oracle, _))| {
                                    let ready = oracle.ready_for_install(self);
                                    if !ready {
                                        debug!(target: "oracle", "oracle `{}` is disabled", oracle.name());
                                    }

                                    (key, (oracle, ready))
                                }).collect();

                                let oracles = self.oracles.clone();
                                let ready_oracles = oracles.into_values().filter(|(_, a)| *a);
                                for (oracle, _) in ready_oracles {
                                    let spy_points = oracle.spy_points();
                                    for request in spy_points {
                                        weak_error!(self.set_transparent_breakpoint(request));
                                    }
                                }

                                // ignore possible signals and watchpoints
                                while self.step_over_breakpoint()?.is_some() {}
                                continue;
                            }
                            BrkptType::LinkerMapFn => {
                                // ignore possible signals and watchpoints
                                while self.step_over_breakpoint()?.is_some() {}
                                print_warns!(self.refresh_deferred());
                                continue;
                            }
                            BrkptType::UserDefined => {
                                let pc = current_pc.into_global(&self.debugee)?;
                                let dwarf = self.debugee.debug_info(self.ecx().location().pc)?;
                                let place = weak_error!(dwarf.find_place_from_pc(pc)).flatten();
                                let func = weak_error!(dwarf.find_function_by_pc(pc))
                                    .flatten()
                                    .map(|(_, info)| info);
                                let tracee_ctl = self.debugee.tracee_ctl();
                                let tracee_in_focus = tracee_ctl
                                    .tracee(self.ecx().pid_on_focus())
                                    .map(|t| t.number);
                                self.hooks
                                    .on_breakpoint(
                                        current_pc,
                                        bp.number(),
                                        place,
                                        func,
                                        tracee_in_focus,
                                    )
                                    .map_err(Hook)?;
                                break event;
                            }
                            BrkptType::WatchpointCompanion(_) => {
                                unreachable!("should not coming from tracer directly");
                            }
                            BrkptType::Temporary | BrkptType::TemporaryAsync => {
                                break event;
                            }
                            BrkptType::Transparent(callback) => {
                                callback.clone()(self);

                                match self.step_over_breakpoint()? {
                                    Some(StopReason::SignalStop(pid, sign)) => {
                                        self.hooks.on_signal(sign);
                                        return Ok(StopReason::SignalStop(pid, sign));
                                    }
                                    Some(StopReason::Watchpoint(pid, addr, ty)) => {
                                        self.execute_on_watchpoint_hook(pid, addr, &ty)?;
                                        return Ok(StopReason::Watchpoint(pid, current_pc, ty));
                                    }
                                    _ => continue,
                                }
                            }
                        }
                    }
                }
                StopReason::SignalStop(pid, sign) => {
                    if !self.debugee.is_in_progress() {
                        continue;
                    }

                    self.ecx_switch_thread(pid)?;
                    self.hooks.on_signal(sign);
                    break event;
                }
                StopReason::Watchpoint(pid, current_pc, ref ty) => {
                    self.ecx_switch_thread(pid)?;
                    self.execute_on_watchpoint_hook(pid, current_pc, ty)?;
                    break event;
                }
            }
        };

        Ok(stop_reason)
    }

    /// Restart debugee by recreating debugee process, save all user-defined breakpoints.
    /// Return when new debugee stopped or ends.
    ///
    /// **! change exploration context**
    pub fn restart_debugee(&mut self) -> Result<Pid, Error> {
        match self.debugee.execution_status() {
            ExecutionStatus::Unload => {
                // all breakpoints and watchpoints already disabled by default
            }
            ExecutionStatus::InProgress => {
                print_warns!(
                    self.watchpoints.clear_local_disable_global(
                        self.debugee.tracee_ctl(),
                        &mut self.breakpoints
                    )
                );
                print_warns!(self.breakpoints.disable_all_breakpoints(&self.debugee)?);
            }
            ExecutionStatus::Exited => {
                // all breakpoints and watchpoints
                // already disabled by [`StopReason::DebugeeExit`] handler
            }
        }

        if !self.debugee.is_exited() {
            let proc_pid = self.process.pid();
            signal::kill(proc_pid, SIGKILL).map_err(|e| Syscall("kill", e))?;
            _ = self
                .debugee
                .tracer_mut()
                .resume(TraceContext::new(&[], &self.watchpoints));
        }

        self.process = self.process.install()?;

        let new_debugee = self.debugee.extend(self.process.pid());
        _ = mem::replace(&mut self.debugee, new_debugee);

        // breakpoints will be enabled later, when StopReason::DebugeeStart state is reached
        self.breakpoints.update_pid(self.process.pid());

        self.hooks.on_process_install(self.process.pid(), None);
        self.expl_context = ExplorationContext::new_non_running(self.process.pid());
        self.continue_execution()?;
        Ok(self.process.pid())
    }

    fn start_debugee_inner(&mut self, force: bool, dry_start: bool) -> Result<(), Error> {
        if dry_start {
            if (self.debugee.is_in_progress() || self.debugee.is_exited()) && !force {
                return Err(Error::AlreadyRun);
            }
            return Ok(());
        }

        match self.debugee.execution_status() {
            ExecutionStatus::Unload => {
                self.continue_execution()?;
            }
            ExecutionStatus::InProgress | ExecutionStatus::Exited if force => {
                self.restart_debugee()?;
            }
            ExecutionStatus::InProgress | ExecutionStatus::Exited => return Err(Error::AlreadyRun),
        };

        Ok(())
    }

    /// Start and execute debugee.
    /// Return when debugee stopped or ends.
    ///
    /// # Errors
    ///
    /// Return error if debugee already run or execution fails.
    pub fn start_debugee(&mut self) -> Result<(), Error> {
        self.start_debugee_inner(false, false)
    }

    /// Start and execute debugee, returning a structured stop reason.
    ///
    /// This API is primarily intended for protocol adapters (e.g. DAP), where the UI needs
    /// a machine-readable reason why the debugee stopped.
    pub fn start_debugee_with_reason(&mut self) -> Result<StopReason, Error> {
        // Reuse existing validation logic and then return the underlying stop reason.
        match self.debugee.execution_status() {
            ExecutionStatus::Unload => self.continue_execution(),
            ExecutionStatus::InProgress | ExecutionStatus::Exited => Err(Error::AlreadyRun),
        }
    }

    /// Start and execute debugee. Restart if debugee already started.
    /// Return when debugee stopped or ends.
    pub fn start_debugee_force(&mut self) -> Result<(), Error> {
        self.start_debugee_inner(true, false)
    }

    /// Start and execute debugee (restart if already started), returning a structured stop reason.
    pub fn start_debugee_force_with_reason(&mut self) -> Result<StopReason, Error> {
        match self.debugee.execution_status() {
            ExecutionStatus::Unload => self.continue_execution(),
            ExecutionStatus::InProgress | ExecutionStatus::Exited => {
                self.restart_debugee()?;
                // restart_debugee itself continues execution until the next stop.
                // If it returns successfully, we are already stopped; map this to a synthetic reason.
                Ok(StopReason::DebugeeStart)
            }
        }
    }

    /// Dry start debugee. Return immediately.
    ///
    /// # Errors
    ///
    /// Return error if debugee already runs.
    pub fn dry_start_debugee(&mut self) -> Result<(), Error> {
        self.start_debugee_inner(false, true)
    }

    /// Continue debugee execution.
    pub fn continue_debugee(&mut self) -> Result<(), Error> {
        disable_when_not_stared!(self);
        self.continue_execution()?;
        Ok(())
    }

    /// Continue debugee execution and return a structured stop reason.
    pub fn continue_debugee_with_reason(&mut self) -> Result<StopReason, Error> {
        disable_when_not_stared!(self);
        self.continue_execution()
    }

    /// Interrupt (pause) execution of the whole debugee process.
    ///
    /// This is used by non-interactive frontends (e.g. DAP) to implement the `pause` request.
    pub fn pause_debugee(&mut self) -> Result<(), Error> {
        let active_bps = self.breakpoints.active_breakpoints();
        self.debugee
            .pause(TraceContext::new(&active_bps, &self.watchpoints))
    }

    /// Return list of symbols matching regular expression.
    ///
    /// # Arguments
    ///
    /// * `regex`: regular expression
    pub fn get_symbols(&'_ self, regex: &str) -> Result<Vec<Symbol<'_>>, Error> {
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
        self.debugee.frame_info(self.ecx())
    }

    /// Set new frame into focus.
    ///
    /// # Arguments
    ///
    /// * `num`: frame number in backtrace
    pub fn set_frame_into_focus(&mut self, num: u32) -> Result<u32, Error> {
        disable_when_not_stared!(self);
        let ecx = self.ecx();
        let backtrace = self.debugee.unwind(ecx.pid_on_focus())?;
        let frame = backtrace.get(num as usize).ok_or(FrameNotFound(num))?;
        self.expl_context = ExplorationContext::new(
            Location {
                pc: frame.ip,
                global_pc: frame.ip.into_global(&self.debugee)?,
                pid: ecx.pid_on_focus(),
            },
            num,
        );
        Ok(num)
    }

    /// Execute `on_step` callback with current exploration context
    fn execute_on_step_hook(&self) -> Result<(), Error> {
        let ecx = self.ecx();
        let pc = ecx.location().pc;
        let global_pc = ecx.location().global_pc;
        let dwarf = self.debugee.debug_info(pc)?;
        let place = weak_error!(dwarf.find_place_from_pc(global_pc)).flatten();
        let func = weak_error!(dwarf.find_function_by_pc(global_pc))
            .flatten()
            .map(|(_, info)| info);
        let tracee_ctl = self.debugee.tracee_ctl();
        let thread_in_focus = tracee_ctl.tracee(ecx.pid_on_focus()).map(|t| t.number);

        self.hooks
            .on_step(pc, place, func, thread_in_focus)
            .map_err(Hook)
    }

    /// Execute `on_async_step` callback with current exploration context
    fn execute_on_async_step_hook(&self, task_id: u64, task_completed: bool) -> Result<(), Error> {
        let ecx = self.ecx();
        let pc = ecx.location().pc;
        let global_pc = ecx.location().global_pc;
        let dwarf = self.debugee.debug_info(pc)?;
        let place = weak_error!(dwarf.find_place_from_pc(global_pc)).flatten();
        let func = weak_error!(dwarf.find_function_by_pc(global_pc))
            .flatten()
            .map(|(_, info)| info);
        self.hooks
            .on_async_step(pc, place, func, task_id, task_completed)
            .map_err(Hook)
    }

    /// Do a single step (until debugee reaches a different source line).
    ///
    /// **! change exploration context**
    pub fn step_into(&mut self) -> Result<(), Error> {
        disable_when_not_stared!(self);
        self.ecx_restore_frame()?;

        match self.step_in()? {
            StepResult::Done => self.execute_on_step_hook(),
            StepResult::SignalInterrupt { signal, quiet } if !quiet => {
                self.hooks.on_signal(signal);
                Ok(())
            }
            StepResult::WatchpointInterrupt {
                pid,
                addr,
                ref ty,
                quiet,
            } if !quiet => self.execute_on_watchpoint_hook(pid, addr, ty),
            _ => Ok(()),
        }
    }

    /// Move in focus thread to the next instruction.
    ///
    /// **! change exploration context**
    pub fn stepi(&mut self) -> Result<(), Error> {
        disable_when_not_stared!(self);
        self.ecx_restore_frame()?;

        match self.single_step_instruction()? {
            Some(StopReason::SignalStop(_, sign)) => {
                self.hooks.on_signal(sign);
                Ok(())
            }
            Some(StopReason::Watchpoint(pid, addr, ref ty)) => {
                self.execute_on_watchpoint_hook(pid, addr, ty)
            }
            _ => self.execute_on_step_hook(),
        }
    }

    /// Return list of currently running debugee threads.
    pub fn thread_state(&self) -> Result<Vec<ThreadSnapshot>, Error> {
        disable_when_not_stared!(self);
        self.debugee.thread_state(self.ecx())
    }

    /// Sets the thread into focus.
    ///
    /// # Arguments
    ///
    /// * `num`: thread number
    pub fn set_thread_into_focus(&mut self, num: u32) -> Result<Tracee, Error> {
        disable_when_not_stared!(self);
        let tracee = self.debugee.get_tracee_by_num(num)?;
        self.ecx_switch_thread(tracee.pid)?;
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

    /// Read N bytes from a debugee process.
    ///
    /// # Arguments
    ///
    /// * `addr`: address in debugee address space where reads
    /// * `read_n`: read byte count
    pub fn read_memory(&self, addr: usize, read_n: usize) -> Result<Vec<u8>, Error> {
        disable_when_not_stared!(self);
        read_memory_by_pid(self.debugee.tracee_ctl().proc_pid(), addr, read_n).map_err(Ptrace)
    }

    /// Write sizeof(uintptr_t) bytes in debugee address space.
    /// Note that little endian byte order will be used when writing.
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
        self.ecx_restore_frame()?;
        self.step_out_frame()?;
        self.execute_on_step_hook()
    }

    /// Do debugee step (over subroutine calls to).
    pub fn step_over(&mut self) -> Result<(), Error> {
        disable_when_not_stared!(self);
        self.ecx_restore_frame()?;
        match self.step_over_any()? {
            StepResult::Done => self.execute_on_step_hook(),
            StepResult::SignalInterrupt { signal, quiet } if !quiet => {
                self.hooks.on_signal(signal);
                Ok(())
            }
            StepResult::WatchpointInterrupt {
                pid,
                addr,
                ref ty,
                quiet,
            } if !quiet => self.execute_on_watchpoint_hook(pid, addr, ty),
            _ => Ok(()),
        }
    }

    /// Reads all local variables from current function in current thread.
    pub fn read_local_variables(&self) -> Result<Vec<QueryResult<'_>>, Error> {
        disable_when_not_stared!(self);

        let executor = variable::execute::DqeExecutor::new(self);
        let eval_result = executor.query(&Dqe::Variable(Selector::Any))?;
        Ok(eval_result)
    }

    /// Reads any variable from the current thread, uses a select expression to filter variables
    /// and fetch their properties (such as structure fields or array elements).
    ///
    /// # Arguments
    ///
    /// * `select_expr`: data query expression
    pub fn read_variable(&self, select_expr: Dqe) -> Result<Vec<QueryResult<'_>>, Error> {
        disable_when_not_stared!(self);
        let executor = variable::execute::DqeExecutor::new(self);
        let eval_result = executor.query(&select_expr)?;
        Ok(eval_result)
    }

    ///  Reads any variable from the current thread, uses a select expression to filter variables
    /// and return their names.
    ///
    /// # Arguments
    ///
    /// * `select_expr`: data query expression
    pub fn read_variable_names(&self, select_expr: Dqe) -> Result<Vec<String>, Error> {
        disable_when_not_stared!(self);
        let executor = variable::execute::DqeExecutor::new(self);
        executor.query_names(&select_expr)
    }

    /// Reads any argument from the current function, uses a select expression to filter variables
    /// and fetch their properties (such as structure fields or array elements).
    ///
    /// # Arguments
    ///
    /// * `select_expr`: data query expression
    pub fn read_argument(&self, select_expr: Dqe) -> Result<Vec<QueryResult<'_>>, Error> {
        disable_when_not_stared!(self);
        let executor = variable::execute::DqeExecutor::new(self);
        let eval_result = executor.query_arguments(&select_expr)?;
        Ok(eval_result)
    }

    /// Reads any argument from the current function, uses a select expression to filter arguments
    /// and return their names.
    ///
    /// # Arguments
    ///
    /// * `select_expr`: data query expression
    pub fn read_argument_names(&self, select_expr: Dqe) -> Result<Vec<String>, Error> {
        disable_when_not_stared!(self);
        let executor = variable::execute::DqeExecutor::new(self);
        executor.query_arguments_names(&select_expr)
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
        Ok(RegisterMap::current(self.ecx().pid_on_focus())?.value(r))
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
            pid: self.ecx().pid_on_focus(),
        };
        Ok(unwinder
            // there is no chance to determine frame number,
            // cause pc may have owned by code outside backtrace,
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

        let in_focus_pid = self.ecx().pid_on_focus();
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
        disable_when_not_stared!(self);
        self.debugee
            .disasm(self.ecx(), &self.breakpoints.active_breakpoints())
    }

    /// Return two place descriptors, at the start and at the end of the current function.
    pub fn current_function_range(&self) -> Result<FunctionRange<'_>, Error> {
        disable_when_not_stared!(self);
        self.debugee.function_range(self.ecx())
    }
}

impl Drop for Debugger {
    fn drop(&mut self) {
        if self.detached {
            return;
        }
        if self.process.is_external() {
            _ = self.breakpoints.disable_all_breakpoints(&self.debugee);
            // drain all watchpoints before terminating the process
            self.watchpoints
                .clear_all(self.debugee.tracee_ctl(), &mut self.breakpoints);

            let current_tids: Vec<Pid> = self
                .debugee
                .tracee_ctl()
                .tracee_iter()
                .map(|t| t.pid)
                .collect();

            if !current_tids.is_empty() {
                current_tids.iter().for_each(|tid| {
                    sys::ptrace::detach(*tid, None).expect("detach debugee");
                });

                signal::kill(self.debugee.tracee_ctl().proc_pid(), Signal::SIGCONT)
                    .expect("kill debugee");
            }

            return;
        }

        match self.debugee.execution_status() {
            ExecutionStatus::Unload => {
                signal::kill(self.debugee.tracee_ctl().proc_pid(), Signal::SIGKILL)
                    .expect("kill debugee");
                waitpid(self.debugee.tracee_ctl().proc_pid(), None).expect("waiting child");
            }
            ExecutionStatus::InProgress => {
                // ignore all possible errors on breakpoints disabling
                _ = self.breakpoints.disable_all_breakpoints(&self.debugee);
                // drain all watchpoints before terminating the process
                self.watchpoints
                    .clear_all(self.debugee.tracee_ctl(), &mut self.breakpoints);

                let current_tids: Vec<Pid> = self
                    .debugee
                    .tracee_ctl()
                    .tracee_iter()
                    .map(|t| t.pid)
                    .collect();

                // todo currently ok only if all threads in group stop
                // continue all threads with SIGSTOP
                let prepare_stopped: Vec<_> = current_tids
                    .into_iter()
                    .filter(|&tid| sys::ptrace::cont(tid, Signal::SIGSTOP).is_ok())
                    .collect();
                let stopped: Vec<_> = prepare_stopped
                    .into_iter()
                    .filter(|&tid| waitpid(tid, None).is_ok())
                    .collect();
                // detach ptrace
                stopped.into_iter().for_each(|tid| {
                    sys::ptrace::detach(tid, None).expect("detach tracee");
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
