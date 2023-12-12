use crate::debugger;
use crate::debugger::process::{Child, Installed};
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
use crossterm::style::Stylize;
use debugger::Error;
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use r#break::Command as BreakpointCommand;
use rustyline::error::ReadlineError;
use rustyline::history::MemHistory;
use rustyline::Editor;
use std::io::{BufRead, BufReader};
use std::process::exit;
use std::sync::atomic::{AtomicBool, Ordering};
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

type BSEditor = Editor<RLHelper, MemHistory>;

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

    pub fn app_already_run(self) -> Self {
        Self {
            already_run: true,
            ..self
        }
    }

    pub fn build_from_process(
        self,
        process: Child<Installed>,
    ) -> anyhow::Result<TerminalApplication> {
        let (control_tx, control_rx) = mpsc::sync_channel::<Control>(0);
        let mut editor = create_editor(PROMT)?;

        let debugee_pid = Arc::new(Mutex::new(Pid::from_raw(-1)));
        let debugger = {
            let debugee_pid = Arc::clone(&debugee_pid);
            Debugger::new(
                process,
                TerminalHook::new(ExternalPrinter::new(&mut editor)?, move |pid| {
                    *debugee_pid.lock().unwrap() = pid;
                }),
            )
        }?;

        if let Some(h) = editor.helper_mut() {
            h.completer
                .lock()
                .unwrap()
                .replace_file_hints(debugger.known_files().cloned())
        }

        Ok(TerminalApplication {
            debugger,
            debugee_pid,
            editor,
            debugee_out: self.debugee_out,
            debugee_err: self.debugee_err,
            control_tx,
            control_rx,
            already_run: self.already_run,
        })
    }

    pub fn build(self, mut debugger: Debugger) -> anyhow::Result<TerminalApplication> {
        let (control_tx, control_rx) = mpsc::sync_channel::<Control>(0);
        let mut editor = create_editor(PROMT)?;

        if let Some(h) = editor.helper_mut() {
            h.completer
                .lock()
                .unwrap()
                .replace_file_hints(debugger.known_files().cloned())
        }

        let debugee_pid = Arc::new(Mutex::new(debugger.process().pid()));
        debugger.set_hook(TerminalHook::new(
            ExternalPrinter::new(&mut editor)?,
            move |pid| {
                *debugee_pid.lock().unwrap() = pid;
            },
        ));

        Ok(TerminalApplication {
            debugee_pid: Arc::new(Mutex::new(debugger.process().pid())),
            debugger,
            editor,
            debugee_out: self.debugee_out,
            debugee_err: self.debugee_err,
            control_tx,
            control_rx,
            already_run: self.already_run,
        })
    }
}

enum Control {
    /// New command from user received
    Cmd(String),
    /// Terminate application
    Terminate,
    /// Switch to TUI mode
    ChangeMode,
}

pub struct TerminalApplication {
    debugger: Debugger,
    /// shared debugee process pid, installed by hook
    debugee_pid: Arc<Mutex<Pid>>,
    editor: BSEditor,
    debugee_out: DebugeeOutReader,
    debugee_err: DebugeeOutReader,
    control_tx: SyncSender<Control>,
    control_rx: Receiver<Control>,
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

        let app_loop = AppLoop {
            debugger: self.debugger,
            control_rx: self.control_rx,
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
            already_run: self.already_run,
        };

        let mut editor = self.editor;
        {
            let control_tx = self.control_tx.clone();
            thread::spawn(move || {
                HELLO_ONCE.call_once(|| {
                    println!("{WELCOME_TEXT}");
                });

                loop {
                    let line = editor.readline(PROMT);
                    match line {
                        Ok(input) => {
                            if input == "q" || input == "quit" {
                                _ = control_tx.send(Control::Terminate);
                                break;
                            } else if input == "tui" {
                                _ = control_tx.send(Control::ChangeMode);
                                break;
                            } else {
                                _ = editor.add_history_entry(&input);
                                _ = control_tx.send(Control::Cmd(input));
                            }
                        }
                        Err(err) => {
                            let on_sign = |sign: Signal| {
                                if self.control_tx.try_send(Control::Terminate).is_err() {
                                    _ = kill(*self.debugee_pid.lock().unwrap(), sign);
                                    _ = self.control_tx.send(Control::Terminate);
                                }
                            };

                            match err {
                                ReadlineError::Eof | ReadlineError::Interrupted => {
                                    on_sign(Signal::SIGINT);
                                    break;
                                }
                                _ => {
                                    println!("error: {:#}", err);
                                    _ = control_tx.send(Control::Terminate);
                                    break;
                                }
                            }
                        }
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
    control_rx: Receiver<Control>,
    printer: ExternalPrinter,
    completer: Arc<Mutex<CommandCompleter>>,
    debugee_out: DebugeeOutReader,
    debugee_err: DebugeeOutReader,
    cancel_output_flag: Arc<AtomicBool>,
    already_run: bool,
}

impl AppLoop {
    fn yes(&self, question: &str) -> bool {
        self.printer.print(question);

        let act = self
            .control_rx
            .recv()
            .expect("unexpected sender disconnect");
        match act {
            Control::Cmd(cmd) => {
                let cmd = cmd.to_lowercase();
                cmd == "y" || cmd == "yes"
            }
            Control::Terminate | Control::ChangeMode => false,
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
                    self.printer.print(format!(
                        "{} = {}",
                        KeywordView::from(var.name()),
                        render_variable_ir(&var, 0)
                    ));
                }),
            Command::PrintArguments(print_arg_command) => ArgumentsHandler::new(&self.debugger)
                .handle(print_arg_command)?
                .into_iter()
                .for_each(|arg| {
                    self.printer.print(format!(
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
                        .and_then(|bt| bt.get(0).map(|f| f.ip.to_string()));

                    self.printer.print(format!(
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

                            self.printer.print(frame_info);
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
                        self.printer.print(format!(
                            "frame #{} ({})",
                            frame.num,
                            FunctionNameView::from(frame.frame.func_name),
                        ));
                        let cfa = AddressView::from(frame.cfa);
                        self.printer.print(format!("cfa: {cfa}"));
                        let ret_addr = AddressView::from(frame.return_addr);
                        self.printer.print(format!("return address: {ret_addr}"));
                    }
                    FrameResult::BroughtIntoFocus(num) => {
                        self.printer.print(format!("switch to #{num}"));
                    }
                }
            }
            Command::Run => {
                if self.already_run {
                    if self.yes("Restart a program? (y or n)") {
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
                        self.printer.print(format!(
                            "{action} {} at {}",
                            bp.number,
                            AddressView::from(bp.addr),
                        ));
                    }
                    Some(place) => {
                        self.printer.print(format!(
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
                            if self.yes(
                                "Add deferred breakpoint for future shared library load? (y or n)",
                            ) {
                                brkpt_cmd = BreakpointCommand::AddDeferred(
                                        brkpt_cmd
                                            .identity()
                                            .expect("unreachable: deferred breakpoint must based on exists breakpoint"),
                                    );
                                continue;
                            }
                        }
                        Err(e) => return Err(e.into()),
                        Ok(ExecutionResult::AddDeferred) => {
                            self.printer.print("Add deferred endpoint")
                        }
                    }
                    break;
                }
            }
            Command::Memory(mem_cmd) => {
                let read = MemoryHandler::new(&self.debugger).handle(mem_cmd)?;
                self.printer.print(format!("{:#016X}", read));
            }
            Command::Register(reg_cmd) => {
                let response = RegisterHandler::new(&self.debugger).handle(&reg_cmd)?;
                response.iter().for_each(|register| {
                    self.printer.print(format!(
                        "{:10} {:#016X}",
                        register.register_name, register.value
                    ));
                });
            }
            Command::Help { reason, command } => {
                if let Some(reason) = reason {
                    self.printer.print(reason);
                }
                self.printer.print(help_for_command(command.as_deref()));
            }
            Command::SkipInput => {}
            Command::PrintSymbol(symbol) => {
                let symbols = SymbolHandler::new(&self.debugger).handle(&symbol)?;
                for symbol in symbols {
                    self.printer.print(format!(
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
                                self.printer.print(format!("{}", view.bold()))
                            } else {
                                self.printer.print(view)
                            }
                        }
                    }
                    ThreadResult::BroughtIntoFocus(thread) => self
                        .printer
                        .print(format!("Thread #{} brought into focus", thread.number)),
                }
            }
            Command::SharedLib => {
                let handler = SharedlibHandler::new(&self.debugger);
                for lib in handler.handle() {
                    let mb_range = lib
                        .range
                        .map(|range| format!("{} - {}", range.from, range.to));

                    self.printer.print(format!(
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
                self.printer.print(format!(
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
                        self.printer.print(format!("{}", instruction_view.bold()));
                    } else {
                        self.printer.print(instruction_view);
                    }
                }
            }
        }

        Ok(())
    }

    fn run(mut self) {
        loop {
            let Ok(action) = self.control_rx.recv() else {
                break;
            };

            match action {
                Control::Cmd(command) => {
                    thread::sleep(Duration::from_millis(1));
                    if let Err(e) = self.handle_command(&command) {
                        match e {
                            CommandError::Parsing(_) => {
                                self.printer.print(ErrorView::from(e));
                            }
                            CommandError::Handle(ref err) if err.is_fatal() => {
                                self.printer.print(ErrorView::from("shutdown debugger"));
                                self.printer
                                    .print(ErrorView::from(format!("fatal debugger error: {e:#}")));
                                exit(0);
                            }
                            CommandError::Handle(_) => {
                                self.printer
                                    .print(ErrorView::from(format!("debugger error: {e:#}")));
                            }
                        }
                    }
                }
                Control::Terminate => {
                    break;
                }
                Control::ChangeMode => {
                    self.cancel_output_flag.store(true, Ordering::SeqCst);
                    let mut tui_builder =
                        crate::ui::tui::AppBuilder::new(self.debugee_out, self.debugee_err);
                    if self.already_run {
                        tui_builder = tui_builder.app_already_run();
                    }
                    let app = tui_builder.build(self.debugger);
                    app.run().expect("run application fail");
                    break;
                }
            }
        }
    }
}
