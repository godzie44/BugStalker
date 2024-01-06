use crate::debugger;
use crate::debugger::variable::render::RenderRepr;
use crate::debugger::variable::select::{Expression, VariableSelector};
use crate::debugger::Debugger;
use crate::ui::command;
use crate::ui::command::arguments::Handler as ArgumentsHandler;
use crate::ui::command::backtrace::Handler as BacktraceHandler;
use crate::ui::command::disasm::Handler as DisAsmHandler;
use crate::ui::command::frame::ExecutionResult as FrameResult;
use crate::ui::command::frame::Handler as FrameHandler;
use crate::ui::command::memory::Handler as MemoryHandler;
use crate::ui::command::r#break::ExecutionResult;
use crate::ui::command::r#break::Handler as BreakpointHandler;
use crate::ui::command::r#continue::Handler as ContinueHandler;
use crate::ui::command::register::Handler as RegisterHandler;
use crate::ui::command::run::Handler as RunHandler;
use crate::ui::command::sharedlib::Handler as SharedlibHandler;
use crate::ui::command::symbol::Handler as SymbolHandler;
use crate::ui::command::thread::ExecutionResult as ThreadResult;
use crate::ui::command::variables::Handler as VariablesHandler;
use crate::ui::command::{r#break, step_instruction, step_into, step_out, step_over, CommandError};
use crate::ui::command::{run, Command};
use crate::ui::console::editor::{create_editor, CommandCompleter, RLHelper};
use crate::ui::console::help::*;
use crate::ui::console::hook::TerminalHook;
use crate::ui::console::print::style::{
    AddressView, AsmInstructionView, AsmOperandsView, ErrorView, FilePathView, FunctionNameView,
    KeywordView,
};
use crate::ui::console::print::ExternalPrinter;
use crate::ui::console::variable::render_variable_ir;
use crate::ui::DebugeeOutReader;
use crossterm::style::{Color, Stylize};
use debugger::Error;
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use r#break::Command as BreakpointCommand;
use rustyline::error::ReadlineError;
use rustyline::history::MemHistory;
use rustyline::Editor;
use std::io::{BufRead, BufReader};
use std::process::exit;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{mpsc, Arc, Mutex, Once};
use std::thread;
use std::time::Duration;
use timeout_readwrite::TimeoutReader;

mod editor;
mod help;
pub mod hook;
pub mod print;
mod variable;
pub mod view;

const WELCOME_TEXT: &str = r#"
BugStalker greets
"#;
const PROMT: &str = "(bs) ";
const PROMT_YES_NO: &str = "(bs y/n) ";

type BSEditor = Editor<RLHelper, MemHistory>;

/// Shared debugee process pid, installed by hook or at console ui creation
static DEBUGEE_PID: AtomicI32 = AtomicI32::new(-1);

pub struct AppBuilder {
    debugee_out: DebugeeOutReader,
    debugee_err: DebugeeOutReader,
    already_run: bool,
}

impl AppBuilder {
    pub fn new(debugee_out: DebugeeOutReader, debugee_err: DebugeeOutReader) -> Self {
        Self {
            debugee_out,
            debugee_err,
            already_run: false,
        }
    }

    pub fn with_already_run(self, already_run: bool) -> Self {
        Self {
            already_run,
            ..self
        }
    }

    pub fn build(self, mut debugger: Debugger) -> anyhow::Result<TerminalApplication> {
        let (user_cmd_tx, user_cmd_rx) = mpsc::sync_channel::<UserAction>(0);
        let oracles = debugger.all_oracles().map(|o| o.name()).collect::<Vec<_>>();
        let mut editor = create_editor(PROMT, &oracles)?;

        if let Some(h) = editor.helper_mut() {
            h.completer
                .lock()
                .unwrap()
                .replace_file_hints(debugger.known_files().cloned())
        }

        DEBUGEE_PID.store(debugger.process().pid().as_raw(), Ordering::Release);
        debugger.set_hook(TerminalHook::new(
            ExternalPrinter::new(&mut editor)?,
            move |pid| DEBUGEE_PID.store(pid.as_raw(), Ordering::Release),
        ));

        Ok(TerminalApplication {
            debugger,
            editor,
            debugee_out: self.debugee_out,
            debugee_err: self.debugee_err,
            user_act_tx: user_cmd_tx,
            user_act_rx: user_cmd_rx,
            already_run: self.already_run,
        })
    }
}

enum UserAction {
    /// New command from user received
    Cmd(String),
    /// Terminate application
    Terminate,
    /// Switch to TUI mode
    ChangeMode,
}

enum EditorMode {
    Default,
    YesNo,
}

pub struct TerminalApplication {
    debugger: Debugger,
    editor: BSEditor,
    debugee_out: DebugeeOutReader,
    debugee_err: DebugeeOutReader,
    user_act_tx: SyncSender<UserAction>,
    user_act_rx: Receiver<UserAction>,
    already_run: bool,
}

pub static HELLO_ONCE: Once = Once::new();

impl TerminalApplication {
    pub fn run(mut self) -> anyhow::Result<()> {
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
            already_run: self.already_run,
        };

        static CTRLC_ONCE: Once = Once::new();
        CTRLC_ONCE.call_once(|| {
            ctrlc::set_handler(move || {
                let pid = Pid::from_raw(DEBUGEE_PID.load(Ordering::Acquire));
                _ = kill(pid, Signal::SIGINT);
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
                                let pid = Pid::from_raw(DEBUGEE_PID.load(Ordering::Acquire));
                                _ = kill(pid, Signal::SIGINT);
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

        app_loop.run();

        Ok(())
    }
}

struct AppLoop {
    debugger: Debugger,
    user_input_rx: Receiver<UserAction>,
    printer: ExternalPrinter,
    completer: Arc<Mutex<CommandCompleter>>,
    debugee_out: DebugeeOutReader,
    debugee_err: DebugeeOutReader,
    cancel_output_flag: Arc<AtomicBool>,
    helper: Helper,
    ready_to_next_command_tx: mpsc::Sender<EditorMode>,
    already_run: bool,
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
                UserAction::Terminate | UserAction::ChangeMode => false,
            };
        }
    }

    fn update_completer_variables(&self) -> anyhow::Result<()> {
        let vars = self
            .debugger
            .read_variable_names(Expression::Variable(VariableSelector::Any))?;
        let args = self
            .debugger
            .read_argument_names(Expression::Variable(VariableSelector::Any))?;

        let mut completer = self.completer.lock().unwrap();
        completer.replace_local_var_hints(vars);
        completer.replace_arg_hints(args);
        Ok(())
    }

    fn handle_command(&mut self, cmd: &str) -> Result<(), CommandError> {
        match Command::parse(cmd)? {
            Command::PrintVariables(print_var_command) => VariablesHandler::new(&self.debugger)
                .handle(print_var_command)?
                .into_iter()
                .for_each(|var| {
                    self.printer.println(format!(
                        "{} = {}",
                        KeywordView::from(var.name()),
                        render_variable_ir(&var, 0)
                    ));
                }),
            Command::PrintArguments(print_arg_command) => ArgumentsHandler::new(&self.debugger)
                .handle(print_arg_command)?
                .into_iter()
                .for_each(|arg| {
                    self.printer.println(format!(
                        "{} = {}",
                        KeywordView::from(arg.name()),
                        render_variable_ir(&arg, 0)
                    ));
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

                    if let Some(bt) = thread.bt {
                        for (frame_num, frame) in bt.into_iter().enumerate() {
                            let fn_name = frame.func_name.clone().unwrap_or_default();

                            let user_bt_end = fn_name == "main"
                                || fn_name.contains("::main")
                                || fn_name.contains("::thread_start");

                            let fn_ip_or_zero = frame.fn_start_ip.unwrap_or_default();

                            let mut frame_info = format!(
                                "#{frame_num} {} - {} ({} + {:#X})",
                                AddressView::from(frame.ip),
                                FunctionNameView::from(frame.func_name),
                                AddressView::from(frame.fn_start_ip),
                                frame.ip.as_u64().saturating_sub(fn_ip_or_zero.as_u64()),
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
            Command::Run => {
                if self.already_run {
                    if self.yes("Restart a program?") {
                        RunHandler::new(&mut self.debugger).handle(run::Command::Restart)?
                    }
                } else {
                    RunHandler::new(&mut self.debugger).handle(run::Command::Start)?;
                    self.already_run = true;
                    _ = self.update_completer_variables();
                }
            }
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
                            brkpts
                                .iter()
                                .for_each(|brkpt| print_bp("New breakpoint", brkpt));
                        }
                        Ok(r#break::ExecutionResult::Removed(brkpts)) => {
                            brkpts
                                .iter()
                                .for_each(|brkpt| print_bp("Remove breakpoint", brkpt));
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
            Command::Memory(mem_cmd) => {
                let read = MemoryHandler::new(&self.debugger).handle(mem_cmd)?;
                self.printer.println(format!("{:#016X}", read));
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
            Command::DisAsm => {
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
            Command::Oracle(name, subcmd) => match self.debugger.get_oracle(&name) {
                None => self
                    .printer
                    .println(ErrorView::from("oracle not found or not ready")),
                Some(oracle) => oracle.print(&self.printer, subcmd.as_deref()),
            },
        }

        Ok(())
    }

    fn run(mut self) {
        loop {
            _ = self.ready_to_next_command_tx.send(EditorMode::Default);

            let Ok(action) = self.user_input_rx.recv() else {
                break;
            };

            match action {
                UserAction::Cmd(command) => {
                    if let Err(e) = self.handle_command(&command) {
                        match e {
                            CommandError::Parsing(_) => {
                                self.printer.println(ErrorView::from(e));
                            }
                            CommandError::Handle(ref err) if err.is_fatal() => {
                                self.printer.println(ErrorView::from("shutdown debugger"));
                                self.printer.println(ErrorView::from(format!(
                                    "fatal debugger error: {e:#}"
                                )));
                                exit(0);
                            }
                            CommandError::Handle(_) => {
                                self.printer
                                    .println(ErrorView::from(format!("debugger error: {e:#}")));
                            }
                        }
                    }
                }
                UserAction::Terminate => {
                    break;
                }
                UserAction::ChangeMode => {
                    self.cancel_output_flag.store(true, Ordering::SeqCst);
                    let tui_builder =
                        crate::ui::tui::AppBuilder::new(self.debugee_out, self.debugee_err)
                            .with_already_run(self.already_run);
                    let app = tui_builder.build(self.debugger);
                    app.run().expect("run application fail");
                    break;
                }
            }
        }
    }
}
