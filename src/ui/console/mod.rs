use std::io::{BufRead, BufReader};
use std::process::exit;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex, Once, mpsc};
use std::time::Duration;
use std::{thread, vec};

use crossterm::style::{Color, Stylize};
use itertools::Itertools;
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use print::style::ImportantView;
use rustyline::error::ReadlineError;
use timeout_readwrite::TimeoutReader;

use r#break::Command as BreakpointCommand;
use debugger::Error;
use trigger::TriggerRegistry;

use super::command::r#async::AsyncCommandResult;
use super::command::r#async::Command as AsyncCommand;
use super::command::trigger::TriggerEvent;
use crate::debugger;
use crate::debugger::process::{Child, Installed};
use crate::debugger::variable::dqe::{Dqe, Selector};
use crate::debugger::{Debugger, DebuggerBuilder};
use crate::ui::command::backtrace::Handler as BacktraceHandler;
use crate::ui::command::r#break::ExecutionResult;
use crate::ui::command::r#break::Handler as BreakpointHandler;
use crate::ui::command::r#continue::Handler as ContinueHandler;
use crate::ui::command::frame::ExecutionResult as FrameResult;
use crate::ui::command::frame::Handler as FrameHandler;
use crate::ui::command::memory::Handler as MemoryHandler;
use crate::ui::command::print::Handler as PrintHandler;
use crate::ui::command::register::Handler as RegisterHandler;
use crate::ui::command::run::Handler as RunHandler;
use crate::ui::command::sharedlib::Handler as SharedlibHandler;
use crate::ui::command::source_code::{DisAsmHandler, FunctionLineRangeHandler};
use crate::ui::command::symbol::Handler as SymbolHandler;
use crate::ui::command::thread::ExecutionResult as ThreadResult;
use crate::ui::command::watch::ExecutionResult as WatchpointExecutionResult;
use crate::ui::command::watch::Handler as WatchpointHandler;
use crate::ui::command::{Command, run};
use crate::ui::command::{
    CommandError, r#break, source_code, step_instruction, step_into, step_out, step_over,
};
use crate::ui::console::r#async::print_backtrace;
use crate::ui::console::r#async::print_backtrace_full;
use crate::ui::console::r#async::print_task_ex;
use crate::ui::console::editor::{BSEditor, CommandCompleter};
use crate::ui::console::file::FileView;
use crate::ui::console::help::*;
use crate::ui::console::hook::TerminalHook;
use crate::ui::console::print::ExternalPrinter;
use crate::ui::console::print::style::{
    AddressView, AsmInstructionView, AsmOperandsView, ErrorView, FilePathView, FunctionNameView,
    KeywordView,
};
use crate::ui::console::variable::render_variable;
use crate::ui::short::Abbreviator;
use crate::ui::{DebugeeOutReader, config};
use crate::ui::{command, supervisor};
use command::trigger::Command as UserCommandTarget;

mod r#async;
mod editor;
pub mod file;
mod help;
pub mod hook;
pub mod print;
mod trigger;
mod variable;

const WELCOME_TEXT: &str = r#"
BugStalker greets
"#;
const PROMT: &str = "(bs) ";
const PROMT_YES_NO: &str = "(bs y/n) ";
const PROMT_USER_PROGRAM: &str = "> ";

/// Shared debugee process pid, installed by hook or at console ui creation
static DEBUGEE_PID: AtomicI32 = AtomicI32::new(-1);

pub struct AppBuilder {
    debugee_out: DebugeeOutReader,
    debugee_err: DebugeeOutReader,
}

impl AppBuilder {
    pub fn new(debugee_out: DebugeeOutReader, debugee_err: DebugeeOutReader) -> Self {
        Self {
            debugee_out,
            debugee_err,
        }
    }

    fn build_inner(
        self,
        oracles: &[&str],
        debugger_lazy: impl FnOnce(TerminalHook) -> anyhow::Result<Debugger>,
    ) -> anyhow::Result<TerminalApplication> {
        let (user_cmd_tx, user_cmd_rx) = mpsc::sync_channel::<UserAction>(0);
        let mut editor = BSEditor::new(PROMT, oracles, config::current().save_history)?;
        let file_view = Rc::new(FileView::new());
        let trigger_reg = Rc::new(TriggerRegistry::default());
        let hook = TerminalHook::new(
            ExternalPrinter::new(&mut editor)?,
            file_view.clone(),
            move |pid| DEBUGEE_PID.store(pid.as_raw(), Ordering::Release),
            trigger_reg.clone(),
        );

        let debugger = debugger_lazy(hook)?;
        if let Some(h) = editor.helper_mut() {
            h.completer
                .lock()
                .unwrap()
                .replace_file_hints(debugger.known_files().cloned())
        }

        Ok(TerminalApplication {
            debugger,
            editor,
            file_view,
            debugee_out: self.debugee_out,
            debugee_err: self.debugee_err,
            user_act_tx: user_cmd_tx,
            user_act_rx: user_cmd_rx,
            trigger_reg,
        })
    }

    /// Create a new debugger using debugger builder.
    /// Create application then.
    ///
    /// # Arguments
    ///
    /// * `dbg_builder`: already configured debugger builder
    /// * `process`: already install debugee process
    pub fn build(
        self,
        dbg_builder: DebuggerBuilder<TerminalHook>,
        process: Child<Installed>,
    ) -> anyhow::Result<TerminalApplication> {
        let oracles = dbg_builder.oracles().map(|o| o.name()).collect::<Vec<_>>();
        let debugger_ctor = |hook| Ok(dbg_builder.with_hooks(hook).build(process)?);
        self.build_inner(&oracles, debugger_ctor)
    }

    /// Extend new application with existed debugger.
    ///
    /// # Arguments
    ///
    /// * `debugger`: already existed debugger
    pub fn extend(self, mut debugger: Debugger) -> anyhow::Result<TerminalApplication> {
        let oracles = debugger.all_oracles().map(|o| o.name()).collect::<Vec<_>>();
        DEBUGEE_PID.store(debugger.process().pid().as_raw(), Ordering::Release);
        let debugger_ctor = move |hook| {
            debugger.set_hook(hook);
            Ok(debugger)
        };

        self.build_inner(&oracles, debugger_ctor)
    }
}

enum UserAction {
    /// New command from user received
    Cmd(String),
    /// Terminate application
    Terminate,
    /// Switch to TUI mode
    ChangeMode,
    /// Do nothing
    Nop,
}

enum EditorMode {
    Default,
    YesNo,
    UserProgram,
}

pub struct TerminalApplication {
    debugger: Debugger,
    editor: BSEditor,
    file_view: Rc<FileView>,
    debugee_out: DebugeeOutReader,
    debugee_err: DebugeeOutReader,
    user_act_tx: SyncSender<UserAction>,
    user_act_rx: Receiver<UserAction>,
    trigger_reg: Rc<TriggerRegistry>,
}

pub static HELLO_ONCE: Once = Once::new();

impl TerminalApplication {
    pub fn run(mut self) -> anyhow::Result<supervisor::ControlFlow> {
        let logger = env_logger::Logger::from_default_env();
        let filter = logger.filter();
        crate::log::LOGGER_SWITCHER.switch(logger, filter);

        macro_rules! print_out {
            ($stream: expr, $format: tt, $printer: expr, $cancel: expr) => {{
                let mut stream = BufReader::new($stream);
                loop {
                    if $cancel.load(Ordering::SeqCst) {
                        return;
                    }

                    let mut line = String::new();
                    let size = match stream.read_line(&mut line) {
                        Ok(size) => size,
                        Err(e) => {
                            if e.kind() == std::io::ErrorKind::TimedOut {
                                continue;
                            }
                            0
                        }
                    };

                    if size == 0 {
                        return;
                    }
                    $printer.print(format!($format, line))
                }
            }};
        }

        // start threads for printing program stdout and stderr
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let cancel1 = cancel.clone();
            let cancel2 = cancel.clone();

            let stdout = TimeoutReader::new(self.debugee_out.clone(), Duration::from_millis(1));
            let stdout_printer = ExternalPrinter::new(&mut self.editor)?;
            thread::spawn(move || print_out!(stdout, "{}", stdout_printer, cancel1));

            let stderr = TimeoutReader::new(self.debugee_err.clone(), Duration::from_millis(1));
            let stderr_printer = ExternalPrinter::new(&mut self.editor)?;
            thread::spawn(move || print_out!(stderr, "\x1b[31m{}", stderr_printer, cancel2));
        };

        let (ready_to_next_command_tx, ready_to_next_command_rx) = mpsc::channel();

        let app_loop = AppLoop {
            debugger: self.debugger,
            file_view: self.file_view,
            user_input_rx: self.user_act_rx,
            completer: Arc::clone(
                &self
                    .editor
                    .helper_mut()
                    .expect("helper must exists")
                    .completer,
            ),
            printer: ExternalPrinter::new(&mut self.editor)?,
            debugee_out: self.debugee_out.clone(),
            debugee_err: self.debugee_err.clone(),
            cancel_output_flag: cancel,
            ready_to_next_command_tx,
            helper: Default::default(),
            trigger_reg: self.trigger_reg,
        };

        static CTRLC_ONCE: Once = Once::new();
        CTRLC_ONCE.call_once(|| {
            // this handler called only if debugee running, otherwise
            // ctrl+c will handle by `readline`
            ctrlc::set_handler(|| {
                // rewrite default handler is good enough
            })
            .expect("error setting Ctrl-C handler")
        });

        let error_printer = ExternalPrinter::new(&mut self.editor)?;
        let mut editor = self.editor;
        {
            let control_tx = self.user_act_tx.clone();
            thread::spawn(move || {
                HELLO_ONCE.call_once(|| {
                    println!("{WELCOME_TEXT}");
                });

                loop {
                    let promt = match ready_to_next_command_rx.recv() {
                        Ok(EditorMode::Default) => PROMT,
                        Ok(EditorMode::YesNo) => PROMT_YES_NO,
                        Ok(EditorMode::UserProgram) => PROMT_USER_PROGRAM,
                        Err(_) => return,
                    };

                    if let Some(editor_helper) = editor.helper_mut() {
                        editor_helper.colored_prompt = format!("{}", promt.with(Color::DarkGreen));
                    }

                    let line = editor.readline(promt);
                    match line {
                        Ok(input) => {
                            if input == "q" || input == "quit" {
                                _ = control_tx.send(UserAction::Terminate);
                                break;
                            } else if input == "tui" {
                                _ = control_tx.send(UserAction::ChangeMode);
                                break;
                            } else {
                                _ = editor.add_history_entry(&input);
                                _ = control_tx.send(UserAction::Cmd(input));
                            }
                        }
                        Err(err) => match err {
                            ReadlineError::Interrupted => {
                                // this branch chosen if SIGINT coming
                                // when debugee stopped
                                // (at breakpoint, for example),
                                // finished,
                                // or not even running
                                _ = control_tx.send(UserAction::Nop);
                            }
                            ReadlineError::Eof => {
                                if self.user_act_tx.try_send(UserAction::Terminate).is_err() {
                                    let pid = Pid::from_raw(DEBUGEE_PID.load(Ordering::Acquire));
                                    _ = kill(pid, Signal::SIGINT);
                                    _ = self.user_act_tx.send(UserAction::Terminate);
                                }
                                break;
                            }
                            _ => {
                                error_printer.println(ErrorView::from(err));
                                _ = control_tx.send(UserAction::Terminate);
                                break;
                            }
                        },
                    }
                }
            });
        }

        app_loop.run()
    }
}

struct AppLoop {
    debugger: Debugger,
    file_view: Rc<FileView>,
    user_input_rx: Receiver<UserAction>,
    printer: ExternalPrinter,
    completer: Arc<Mutex<CommandCompleter>>,
    debugee_out: DebugeeOutReader,
    debugee_err: DebugeeOutReader,
    cancel_output_flag: Arc<AtomicBool>,
    helper: Helper,
    ready_to_next_command_tx: mpsc::Sender<EditorMode>,
    trigger_reg: Rc<TriggerRegistry>,
}

impl AppLoop {
    fn yes(&self, question: &str) -> bool {
        self.printer.println(question);

        loop {
            _ = self.ready_to_next_command_tx.send(EditorMode::YesNo);
            let act = self
                .user_input_rx
                .recv()
                .expect("unexpected sender disconnect");
            return match act {
                UserAction::Cmd(cmd) => match cmd.to_lowercase().as_str() {
                    "y" | "yes" => true,
                    "n" | "no" => false,
                    _ => continue,
                },
                UserAction::Terminate | UserAction::ChangeMode | UserAction::Nop => false,
            };
        }
    }

    fn take_user_command_list(&self, help: &str) -> Result<trigger::UserProgram, CommandError> {
        self.printer.println(help);
        let mut result = vec![];
        loop {
            _ = self.ready_to_next_command_tx.send(EditorMode::UserProgram);
            let act = self
                .user_input_rx
                .recv()
                .expect("unexpected sender disconnect");
            match act {
                UserAction::Cmd(input) => {
                    if input.as_str().trim() == "end" {
                        break;
                    }

                    let cmd = Command::parse(&input)?;
                    match cmd {
                        Command::Print(_)
                        | Command::PrintBacktrace(_)
                        | Command::Frame(_)
                        | Command::PrintSymbol(_)
                        | Command::Memory(_)
                        | Command::Register(_)
                        | Command::Thread(_)
                        | Command::SharedLib
                        | Command::SourceCode(_)
                        | Command::Oracle(_, _)
                        | Command::Async(AsyncCommand::FullBacktrace)
                        | Command::Async(AsyncCommand::ShortBacktrace)
                        | Command::Async(AsyncCommand::CurrentTask(_)) => {
                            result.push((cmd, input));
                            continue;
                        }
                        _ => {
                            self.printer
                                .println("unsupported command, try another one or `end`");
                            continue;
                        }
                    }
                }

                UserAction::Terminate | UserAction::ChangeMode | UserAction::Nop => {
                    self.printer
                        .println("unsupported command, try another one or `end`");
                    continue;
                }
            };
        }

        Ok(result)
    }

    fn update_completer_variables(&self) -> anyhow::Result<()> {
        let vars = self
            .debugger
            .read_variable_names(Dqe::Variable(Selector::Any))?;
        let args = self
            .debugger
            .read_argument_names(Dqe::Variable(Selector::Any))?;

        let mut completer = self.completer.lock().unwrap();
        completer.replace_local_var_hints(vars);
        completer.replace_arg_hints(args);
        Ok(())
    }

    fn handle_command_str(&mut self, cmd: &str) -> Result<(), CommandError> {
        if cmd.is_empty() {
            return Ok(());
        }

        self.handle_command(Command::parse(cmd)?)
    }

    fn handle_command(&mut self, cmd: Command) -> Result<(), CommandError> {
        match cmd {
            Command::Print(print_var_command) => PrintHandler::new(&self.debugger)
                .handle(print_var_command)?
                .into_iter()
                .for_each(|var| {
                    let string_to_render = match var {
                        command::print::ReadVariableResult::PreRender(qr, val) => {
                            render_variable(&qr, Some(&val))
                        }
                        command::print::ReadVariableResult::Raw(qr) => render_variable(&qr, None),
                    }
                    .unwrap_or(print::style::UNKNOWN_PLACEHOLDER.to_string());
                    self.printer.println(string_to_render)
                }),
            Command::PrintBacktrace(cmd) => {
                let bt = BacktraceHandler::new(&self.debugger).handle(cmd)?;
                bt.into_iter().for_each(|thread| {
                    let ip = thread
                        .bt
                        .as_ref()
                        .and_then(|bt| bt.first().map(|f| f.ip.to_string()));

                    self.printer.println(format!(
                        "thread #{}, {} - {}",
                        thread.thread.number,
                        thread.thread.pid,
                        AddressView::from(ip),
                    ));

                    let abbreviator = Abbreviator::new("/", "/..", 30);

                    if let Some(bt) = thread.bt {
                        for (frame_num, frame) in bt.into_iter().enumerate() {
                            let fn_name = frame.func_name.clone().unwrap_or_default();

                            let user_bt_end = fn_name == "main"
                                //|| fn_name.contains("::main")
                                || fn_name.contains("::thread_start");

                            let place = frame.place;

                            let file_and_line = place
                                .map(|p| {
                                    format!(
                                        "at {}:{}",
                                        FilePathView::from(
                                            abbreviator.apply(&p.file.to_string_lossy())
                                        ),
                                        p.line_number
                                    )
                                })
                                .unwrap_or_default();

                            let mut frame_info = format!(
                                "#{frame_num} {} - {} {}",
                                AddressView::from(frame.ip),
                                FunctionNameView::from(frame.func_name),
                                file_and_line
                            );
                            if thread.focus_frame == Some(frame_num) {
                                frame_info = frame_info.bold().to_string();
                            }

                            self.printer.println(frame_info);
                            if user_bt_end {
                                break;
                            }
                        }
                    }
                });
            }
            Command::Continue => {
                ContinueHandler::new(&mut self.debugger).handle()?;
                _ = self.update_completer_variables();
            }
            Command::Frame(cmd) => {
                let result = FrameHandler::new(&mut self.debugger).handle(cmd)?;
                match result {
                    FrameResult::FrameInfo(frame) => {
                        self.printer.println(format!(
                            "frame #{} ({})",
                            frame.num,
                            FunctionNameView::from(frame.frame.func_name),
                        ));
                        let cfa = AddressView::from(frame.cfa);
                        self.printer.println(format!("cfa: {cfa}"));
                        let ret_addr = AddressView::from(frame.return_addr);
                        self.printer.println(format!("return address: {ret_addr}"));
                    }
                    FrameResult::BroughtIntoFocus(num) => {
                        self.printer.println(format!("switch to #{num}"));
                    }
                }
            }
            Command::Run => match RunHandler::new(&mut self.debugger).handle(run::Command::Start) {
                Err(CommandError::Handle(Error::AlreadyRun)) => {
                    if self.yes("Restart a program?") {
                        RunHandler::new(&mut self.debugger).handle(run::Command::Restart)?
                    }
                }
                Err(e) => return Err(e),
                _ => {
                    _ = self.update_completer_variables();
                }
            },
            Command::StepInstruction => {
                step_instruction::Handler::new(&mut self.debugger).handle()?;
                _ = self.update_completer_variables();
            }
            Command::StepInto => {
                step_into::Handler::new(&mut self.debugger).handle()?;
                _ = self.update_completer_variables();
            }
            Command::StepOut => {
                step_out::Handler::new(&mut self.debugger).handle()?;
                _ = self.update_completer_variables();
            }
            Command::StepOver => {
                step_over::Handler::new(&mut self.debugger).handle()?;
                _ = self.update_completer_variables();
            }
            Command::Breakpoint(mut brkpt_cmd) => {
                let print_bp = |action: &str, bp: &debugger::BreakpointView| match &bp.place {
                    None => {
                        self.printer.println(format!(
                            "{action} {} at {}",
                            bp.number,
                            AddressView::from(bp.addr),
                        ));
                    }
                    Some(place) => {
                        self.printer.println(format!(
                            "{action} {} at {}: {}:{} ",
                            bp.number,
                            AddressView::from(place.address),
                            FilePathView::from(place.file.to_string_lossy()),
                            place.line_number,
                        ));
                    }
                };

                loop {
                    match BreakpointHandler::new(&mut self.debugger).handle(&brkpt_cmd) {
                        Ok(r#break::ExecutionResult::New(brkpts)) => {
                            brkpts.iter().for_each(|brkpt| {
                                print_bp("New breakpoint", brkpt);
                                self.trigger_reg.set_previous_brkpt(brkpt.number);
                            });
                        }
                        Ok(r#break::ExecutionResult::Removed(brkpts)) => {
                            brkpts.iter().for_each(|brkpt| {
                                print_bp("Removed breakpoint", brkpt);
                                self.trigger_reg
                                    .remove(TriggerEvent::Breakpoint(brkpt.number));
                            });
                        }
                        Ok(r#break::ExecutionResult::Dump(brkpts)) => brkpts
                            .iter()
                            .for_each(|brkpt| print_bp("- Breakpoint", brkpt)),
                        Err(Error::NoSuitablePlace) => {
                            if self.yes("Add deferred breakpoint for future shared library load?") {
                                brkpt_cmd = BreakpointCommand::AddDeferred(
                                    brkpt_cmd
                                        .identity()
                                        .expect("unreachable: deferred breakpoint must based on exists breakpoint"),
                                );
                                continue;
                            }
                        }
                        Ok(ExecutionResult::AddDeferred) => {
                            self.printer.println("Add deferred endpoint")
                        }
                        Err(e) => return Err(e.into()),
                    }
                    break;
                }
            }
            Command::Watchpoint(cmd) => {
                let print_wp = |prefix: &str, wp: debugger::WatchpointView| {
                    let source_expr = wp
                        .source_dqe
                        .map(|dqe| format!(", expression: {dqe}"))
                        .unwrap_or_default();
                    let (addr, cond) = (
                        AddressView::from(wp.address),
                        KeywordView::from(wp.condition),
                    );
                    self.printer.println(format!(
                        "{prefix} {} at {}, condition: {}, watch size: {}{source_expr}",
                        wp.number, addr, cond, wp.size
                    ))
                };

                let mut handler = WatchpointHandler::new(&mut self.debugger);
                let res = handler.handle(cmd)?;
                match res {
                    WatchpointExecutionResult::New(wp) => {
                        self.trigger_reg.set_previous_wp(wp.number);
                        print_wp("New watchpoint", wp);
                    }
                    WatchpointExecutionResult::Removed(Some(wp)) => {
                        self.trigger_reg.remove(TriggerEvent::Watchpoint(wp.number));
                        print_wp("Removed watchpoint", wp);
                    }
                    WatchpointExecutionResult::Removed(_) => {
                        self.printer.println("No watchpoint found")
                    }
                    WatchpointExecutionResult::Dump(wps) => {
                        self.printer
                            .println(format!("{}/4 active watchpoints:", wps.len()));
                        for wp in wps {
                            print_wp("- Watchpoint", wp)
                        }
                    }
                }
            }
            Command::Memory(mem_cmd) => {
                let read = MemoryHandler::new(&self.debugger).handle(mem_cmd)?;
                for b in read {
                    self.printer.print(format!("0x{b:X} "));
                }
                self.printer.println("");
            }
            Command::Register(reg_cmd) => {
                let response = RegisterHandler::new(&self.debugger).handle(&reg_cmd)?;
                response.iter().for_each(|register| {
                    self.printer.println(format!(
                        "{:10} {:#016X}",
                        register.register_name, register.value
                    ));
                });
            }
            Command::Help { reason, command } => {
                if let Some(reason) = reason {
                    self.printer.println(reason);
                }
                self.printer.println(
                    self.helper
                        .help_for_command(&self.debugger, command.as_deref()),
                );
            }
            Command::SkipInput => {}
            Command::PrintSymbol(symbol) => {
                let symbols = SymbolHandler::new(&self.debugger).handle(&symbol)?;
                for symbol in symbols {
                    self.printer.println(format!(
                        "{} - {:?} {}",
                        symbol.name,
                        symbol.kind,
                        AddressView::from(symbol.addr)
                    ));
                }
            }
            Command::Thread(cmd) => {
                let result = command::thread::Handler::new(&mut self.debugger).handle(cmd)?;
                match result {
                    ThreadResult::List(mut list) => {
                        list.sort_by(|t1, t2| t1.thread.number.cmp(&t2.thread.number));
                        for thread in list {
                            let current_frame = thread.bt.and_then(|mut bt| bt.drain(..).next());
                            let ip = current_frame.as_ref().map(|f| f.ip.to_string());
                            let func = current_frame.and_then(|f| f.func_name);

                            let view = format!(
                                "#{} thread id: {}, {} in {}",
                                thread.thread.number,
                                thread.thread.pid,
                                AddressView::from(ip),
                                FunctionNameView::from(func),
                            );

                            if thread.in_focus {
                                self.printer.println(format!("{}", view.bold()))
                            } else {
                                self.printer.println(view)
                            }
                        }
                    }
                    ThreadResult::BroughtIntoFocus(thread) => self
                        .printer
                        .println(format!("Thread #{} brought into focus", thread.number)),
                }
            }
            Command::SharedLib => {
                let handler = SharedlibHandler::new(&self.debugger);
                for lib in handler.handle() {
                    let mb_range = lib
                        .range
                        .map(|range| format!("{} - {}", range.from, range.to));

                    self.printer.println(format!(
                        "{}  {}  {}",
                        AddressView::from(mb_range),
                        if !lib.has_debug_info { "*" } else { " " },
                        FilePathView::from(lib.path.to_string_lossy())
                    ))
                }
            }
            Command::SourceCode(inner_cmd) => match inner_cmd {
                source_code::Command::Range(bounds) => {
                    let handler = FunctionLineRangeHandler::new(&self.debugger);
                    let range = handler.handle()?;

                    self.printer.println(format!(
                        "{} at {}:{}",
                        FunctionNameView::from(range.name),
                        FilePathView::from(range.stop_place.file.to_string_lossy()),
                        range.stop_place.line_number,
                    ));

                    self.printer.print(
                        self.file_view
                            .render_source(&range.stop_place, bounds)
                            .map_err(CommandError::FileRender)?,
                    );
                }
                source_code::Command::Function => {
                    let handler = FunctionLineRangeHandler::new(&self.debugger);
                    let range = handler.handle()?;

                    self.printer.println(format!(
                        "{} at {}:{}",
                        FunctionNameView::from(range.name),
                        FilePathView::from(range.stop_place.file.to_string_lossy()),
                        range.stop_place.line_number,
                    ));

                    self.printer.print(
                        self.file_view
                            .render_source_range(range.file, range.start_line, range.end_line)
                            .map_err(CommandError::FileRender)?,
                    );
                }
                source_code::Command::Asm => {
                    let handler = DisAsmHandler::new(&self.debugger);
                    let assembly = handler.handle()?;
                    self.printer.println(format!(
                        "Assembler code for function {}",
                        FunctionNameView::from(assembly.name)
                    ));
                    for ins in assembly.instructions {
                        let instruction_view = format!(
                            "{} {} {}",
                            AddressView::from(ins.address),
                            AsmInstructionView::from(ins.mnemonic),
                            AsmOperandsView::from(ins.operands),
                        );

                        if ins.address == assembly.addr_in_focus {
                            self.printer.println(format!("{}", instruction_view.bold()));
                        } else {
                            self.printer.println(instruction_view);
                        }
                    }
                }
            },
            Command::Async(cmd) => {
                let mut handler = command::r#async::Handler::new(&mut self.debugger);
                let result: command::r#async::AsyncCommandResult = handler.handle(&cmd)?;

                match result {
                    AsyncCommandResult::ShortBacktrace(bt) => {
                        print_backtrace(&bt, &self.printer);
                    }
                    AsyncCommandResult::FullBacktrace(bt) => {
                        print_backtrace_full(&bt, &self.printer);
                    }
                    AsyncCommandResult::CurrentTask(bt, regex) => {
                        print_task_ex(&bt, &self.printer, regex);
                    }
                    AsyncCommandResult::StepOver => {
                        _ = self.update_completer_variables();
                    }
                    AsyncCommandResult::StepOut => {
                        _ = self.update_completer_variables();
                    }
                }
            }
            Command::Trigger(cmd) => {
                let event = match cmd {
                    UserCommandTarget::AttachToPreviouslyCreated => {
                        let Some(event) = self.trigger_reg.get_previous_event() else {
                            self.printer.println(ErrorView::from(
                                "No previously added watchpoints or breakpoints exist",
                            ));
                            return Ok(());
                        };
                        event
                    }
                    UserCommandTarget::AttachToDefined(event) => event,
                    UserCommandTarget::Info => {
                        self.printer.println(format!("{:<30}  Program", "Event"));

                        self.trigger_reg.for_each_trigger(|event, program| {
                            self.printer.println(format!(
                                "{:<30}  {}",
                                event.to_string(),
                                KeywordView::from(program.iter().map(|(_, str)| str).join(", ")),
                            ));
                        });
                        return Ok(());
                    }
                };

                let help = match event {
                    TriggerEvent::Breakpoint(num) => format!("Print the commands to be executed on breakpoint {num}"),
                    TriggerEvent::Watchpoint(num) => format!("Print the commands to be executed on watchpoint {num}"),
                    TriggerEvent::Any => "Print the commands to be executed on each breakpoint or watchpoint, and print 'end' for end".to_string(),
                };

                let commands = self.take_user_command_list(&help)?;
                self.trigger_reg.add(event, commands);
            }
            Command::Call(call) => {
                let mut handler = command::call::Handler::new(&mut self.debugger);
                handler.handle(call)?;
            }
            Command::Oracle(name, subcmd) => match self.debugger.get_oracle(&name) {
                None => self
                    .printer
                    .println(ErrorView::from("Oracle not found or not ready")),
                Some(oracle) => oracle.print(&self.printer, subcmd.as_deref()),
            },
        }

        Ok(())
    }

    fn handle_error(&self, error: CommandError) {
        match error {
            CommandError::Parsing(pretty_error) => {
                self.printer.println(pretty_error);
            }
            CommandError::FileRender(_) => {
                self.printer
                    .println(ErrorView::from(format!("Render file error: {error:#}")));
            }
            CommandError::Handle(ref err) if err.is_fatal() => {
                self.printer.println(ErrorView::from("Shutdown debugger"));
                self.printer
                    .println(ErrorView::from(format!("Fatal error: {error:#}")));
                exit(1);
            }
            CommandError::Handle(_) => {
                self.printer
                    .println(ErrorView::from(format!("Error: {error:#}")));
            }
        }
    }

    fn run(mut self) -> anyhow::Result<supervisor::ControlFlow> {
        loop {
            if let Some(user_program) = self.trigger_reg.take_program() {
                self.printer
                    .println(ImportantView::from("Related program found:"));
                user_program.into_iter().for_each(|(cmd, _)| {
                    if let Err(e) = self.handle_command(cmd) {
                        self.handle_error(e);
                    }
                });
            };

            _ = self.ready_to_next_command_tx.send(EditorMode::Default);

            let Ok(action) = self.user_input_rx.recv() else {
                return Ok(supervisor::ControlFlow::Exit);
            };

            match action {
                UserAction::Cmd(command) => {
                    if let Err(e) = self.handle_command_str(&command) {
                        self.handle_error(e);
                    }
                }
                UserAction::Nop => {}
                UserAction::Terminate => {
                    return Ok(supervisor::ControlFlow::Exit);
                }
                UserAction::ChangeMode => {
                    self.cancel_output_flag.store(true, Ordering::SeqCst);
                    let tui_builder =
                        crate::ui::tui::AppBuilder::new(self.debugee_out, self.debugee_err);
                    let app = tui_builder.extend(self.debugger);
                    return Ok(supervisor::ControlFlow::Switch(
                        supervisor::Application::TUI(app),
                    ));
                }
            }
        }
    }
}
