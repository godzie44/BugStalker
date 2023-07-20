use super::debugger::command::Continue;
use crate::console::editor::{create_editor, CommandCompleter, RLHelper};
use crate::console::help::*;
use crate::console::hook::TerminalHook;
use crate::console::print::ExternalPrinter;
use crate::console::variable::render_variable_ir;
use crate::debugger;
use crate::debugger::command::{
    Arguments, Backtrace, Break, BreakpointHandlingResult, Command, Frame, Run, StepI, StepInto,
    StepOut, StepOver, Symbol, Variables,
};
use crate::debugger::process::{Child, Installed};
use crate::debugger::variable::render::RenderRepr;
use crate::debugger::variable::select::{Expression, VariableSelector};
use crate::debugger::{command, Debugger};
use command::{Memory, Register};
use crossterm::style::Stylize;
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use os_pipe::PipeReader;
use rustyline::error::ReadlineError;
use rustyline::history::MemHistory;
use rustyline::Editor;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

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
    debugee_out: PipeReader,
    debugee_err: PipeReader,
}

impl AppBuilder {
    pub fn new(debugee_out: PipeReader, debugee_err: PipeReader) -> Self {
        Self {
            debugee_out,
            debugee_err,
        }
    }

    pub fn build(self, process: Child<Installed>) -> anyhow::Result<TerminalApplication> {
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
            debugee_out: Arc::new(self.debugee_out),
            debugee_err: Arc::new(self.debugee_err),
            control_tx,
            control_rx,
        })
    }
}

enum Control {
    Cmd(String),
    Terminate,
}

pub struct TerminalApplication {
    debugger: Debugger,
    /// shared debugee process pid, installed by hook
    debugee_pid: Arc<Mutex<Pid>>,
    editor: BSEditor,
    debugee_out: Arc<PipeReader>,
    debugee_err: Arc<PipeReader>,
    control_tx: SyncSender<Control>,
    control_rx: Receiver<Control>,
}

impl TerminalApplication {
    pub fn run(mut self) -> anyhow::Result<()> {
        env_logger::init();

        macro_rules! print_out {
            ($stream: expr, $format: tt, $printer: expr) => {{
                let mut stream = BufReader::new($stream);
                loop {
                    let mut line = String::new();
                    let size = stream.read_line(&mut line).unwrap_or(0);
                    if size == 0 {
                        return;
                    }
                    $printer.print(format!($format, line))
                }
            }};
        }

        // start threads for printing program stdout and stderr
        {
            let stdout = self.debugee_out.clone();
            let stdout_printer = ExternalPrinter::new(&mut self.editor)?;
            thread::spawn(move || print_out!(stdout.as_ref(), "{}", stdout_printer));

            let stderr = self.debugee_err.clone();
            let stderr_printer = ExternalPrinter::new(&mut self.editor)?;
            thread::spawn(move || print_out!(stderr.as_ref(), "\x1b[31m{}", stderr_printer));
        }

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
        };

        let mut editor = self.editor;
        {
            let control_tx = self.control_tx.clone();
            thread::spawn(move || {
                println!("{WELCOME_TEXT}");
                loop {
                    let line = editor.readline(PROMT);
                    match line {
                        Ok(input) => {
                            if input == "q" || input == "quit" {
                                _ = control_tx.send(Control::Terminate);
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
}

impl AppLoop {
    fn yes(&mut self, question: &str) -> anyhow::Result<bool> {
        self.printer.print(question);

        let act = self.control_rx.recv()?;
        match act {
            Control::Cmd(cmd) => {
                let cmd = cmd.to_lowercase();
                Ok(cmd == "y" || cmd == "yes")
            }
            Control::Terminate => Ok(false),
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

    fn handle_command(&mut self, cmd: &str) -> anyhow::Result<()> {
        match Command::parse(cmd)? {
            Command::PrintVariables(print_var_command) => Variables::new(&self.debugger)
                .handle(print_var_command)?
                .into_iter()
                .for_each(|var| {
                    self.printer
                        .print(format!("{} = {}", var.name(), render_variable_ir(&var, 0)));
                }),
            Command::PrintArguments(print_arg_command) => Arguments::new(&self.debugger)
                .handle(print_arg_command)?
                .into_iter()
                .for_each(|arg| {
                    self.printer
                        .print(format!("{} = {}", arg.name(), render_variable_ir(&arg, 0)));
                }),
            Command::PrintBacktrace(cmd) => {
                let bt = Backtrace::new(&self.debugger).handle(cmd)?;
                bt.iter().for_each(|thread| {
                    self.printer.print(format!(
                        "thread {} - {}",
                        thread.thread.pid,
                        thread
                            .bt
                            .as_ref()
                            .and_then(|bt| bt.get(0).map(|f| f.ip))
                            .unwrap_or(0_usize.into())
                    ));

                    if let Some(ref bt) = thread.bt {
                        for frame in bt.iter() {
                            match &frame.func_name {
                                None => {
                                    self.printer.print(format!("{} - ????", frame.ip));
                                }
                                Some(fn_name) => {
                                    let user_bt_end = fn_name == "main"
                                        || fn_name.contains("::main")
                                        || fn_name.contains("::thread_start");

                                    let fn_ip = frame.fn_start_ip.unwrap_or_default();
                                    self.printer.print(format!(
                                        "{} - {} ({} + {:#X})",
                                        frame.ip,
                                        fn_name,
                                        fn_ip,
                                        frame.ip.as_u64() - fn_ip.as_u64(),
                                    ));

                                    if user_bt_end {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                });
            }
            Command::Continue => {
                Continue::new(&mut self.debugger).handle()?;
                _ = self.update_completer_variables();
            }
            Command::PrintFrame => {
                let frame = Frame::new(&self.debugger).handle()?;
                self.printer.print(format!("cfa: {}", frame.cfa));
                self.printer.print(format!(
                    "return address: {}",
                    frame
                        .return_addr
                        .map_or(String::from("unknown"), |addr| format!("{}", addr))
                ));
            }
            Command::Run => {
                static ALREADY_RUN: AtomicBool = AtomicBool::new(false);

                if ALREADY_RUN.load(Ordering::Acquire) {
                    if self.yes("Restart program? (y or n)")? {
                        Run::new(&mut self.debugger).restart()?
                    }
                } else {
                    Run::new(&mut self.debugger).start()?;
                    ALREADY_RUN.store(true, Ordering::Release);
                    _ = self.update_completer_variables();
                }
            }
            Command::StepInstruction => {
                StepI::new(&mut self.debugger).handle()?;
                _ = self.update_completer_variables();
            }
            Command::StepInto => {
                StepInto::new(&mut self.debugger).handle()?;
                _ = self.update_completer_variables();
            }
            Command::StepOut => {
                StepOut::new(&mut self.debugger).handle()?;
                _ = self.update_completer_variables();
            }
            Command::StepOver => {
                StepOver::new(&mut self.debugger).handle()?;
                _ = self.update_completer_variables();
            }
            Command::Breakpoint(bp_cmd) => {
                let print_bp = |action: &str, bp: &debugger::BreakpointView| match &bp.place {
                    None => {
                        self.printer.print(format!(
                            "{action} {} at {}",
                            bp.number,
                            bp.addr.to_string().blue()
                        ));
                    }
                    Some(place) => {
                        self.printer.print(format!(
                            "{action} {} at {}: {}:{} ",
                            bp.number,
                            place.address.to_string().blue(),
                            place.file.to_str().unwrap_or_default().green(),
                            place.line_number,
                        ));
                    }
                };

                match Break::new(&mut self.debugger).handle(bp_cmd)? {
                    BreakpointHandlingResult::New(bp) => {
                        print_bp("new breakpoint", &bp);
                    }
                    BreakpointHandlingResult::Removed(mb_bp) => match mb_bp {
                        None => self.printer.print("Breakpoint not found"),
                        Some(ref bp) => {
                            print_bp("remove breakpoint", bp);
                        }
                    },
                    BreakpointHandlingResult::Dump(brkpts) => brkpts
                        .iter()
                        .for_each(|brkpt| print_bp("- Breakpoint", brkpt)),
                }
            }
            Command::Memory(mem_cmd) => {
                let read = Memory::new(&self.debugger).handle(mem_cmd)?;
                self.printer.print(format!("{:#016X}", read));
            }
            Command::Register(reg_cmd) => {
                let response = Register::new(&self.debugger).handle(&reg_cmd)?;
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
            Command::PrintSymbol(symbol) => {
                let symbols = Symbol::new(&self.debugger).handle(&symbol)?;
                for symbol in symbols {
                    self.printer.print(format!(
                        "{} - {:?} {}",
                        symbol.name, symbol.kind, symbol.addr
                    ));
                }
            }
        }

        Ok(())
    }

    fn run(mut self) {
        loop {
            let Ok(action) = self.control_rx.recv() else {
                break
            };

            match action {
                Control::Cmd(command) => {
                    thread::sleep(Duration::from_millis(1));
                    if let Err(e) = self.handle_command(&command) {
                        self.printer.print(format!("error: {:#}", e));
                    }
                }
                Control::Terminate => {
                    break;
                }
            }
        }
    }
}
