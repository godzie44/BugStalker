use super::debugger::command::Continue;
use crate::console::hook::TerminalHook;
use crate::console::print::ExternalPrinter;
use crate::console::variable::render_variable_ir;
use crate::debugger::command::{
    Arguments, Backtrace, Break, Command, Frame, Run, StepI, StepInto, StepOut, StepOver, Symbol,
    Variables,
};
use crate::debugger::process::{Child, Installed};
use crate::debugger::variable::render::RenderRepr;
use crate::debugger::{command, Debugger};
use command::{Memory, Register};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use os_pipe::PipeReader;
use rustyline::error::ReadlineError;
use rustyline::highlight::{Highlighter, MatchingBracketHighlighter};
use rustyline::hint::HistoryHinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::MatchingBracketValidator;
use rustyline::{CompletionType, Config, Editor};
use rustyline_derive::{Completer, Helper, Hinter, Validator};
use std::borrow::Cow;
use std::borrow::Cow::{Borrowed, Owned};
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

pub mod hook;
pub mod print;
mod variable;
pub mod view;

const WELCOME_TEXT: &str = r#"
BugStalker greets
"#;
const HISTORY_FILE: &str = "bsh.hist";

#[derive(Helper, Completer, Hinter, Validator)]
struct RLHelper {
    #[rustyline(Completer)]
    completer: (),
    highlighter: MatchingBracketHighlighter,
    #[rustyline(Validator)]
    validator: MatchingBracketValidator,
    #[rustyline(Hinter)]
    hinter: HistoryHinter,
    colored_prompt: String,
}

impl Highlighter for RLHelper {
    fn highlight<'l>(&self, line: &'l str, pos: usize) -> Cow<'l, str> {
        self.highlighter.highlight(line, pos)
    }

    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        default: bool,
    ) -> Cow<'b, str> {
        if default {
            Borrowed(&self.colored_prompt)
        } else {
            Borrowed(prompt)
        }
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Owned("\x1b[1m".to_owned() + hint + "\x1b[m")
    }

    fn highlight_char(&self, line: &str, pos: usize) -> bool {
        self.highlighter.highlight_char(line, pos)
    }
}

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

    fn create_editor(&self) -> anyhow::Result<Editor<RLHelper, DefaultHistory>> {
        let config = Config::builder()
            .history_ignore_space(true)
            .completion_type(CompletionType::List)
            .build();

        let h = RLHelper {
            completer: (),
            highlighter: MatchingBracketHighlighter::new(),
            hinter: HistoryHinter {},
            colored_prompt: "".to_owned(),
            validator: MatchingBracketValidator::new(),
        };

        let mut editor = Editor::with_config(config)?;
        editor.set_helper(Some(h));

        Ok(editor)
    }

    pub fn build(self, process: Child<Installed>) -> anyhow::Result<TerminalApplication> {
        let (control_tx, control_rx) = mpsc::sync_channel::<Control>(0);
        let mut editor = self.create_editor()?;

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
    editor: Editor<RLHelper, DefaultHistory>,
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

        let external_printer = ExternalPrinter::new(&mut self.editor)?;

        {
            let mut editor = self.editor;
            let control_tx = self.control_tx.clone();
            thread::spawn(move || {
                println!("{WELCOME_TEXT}");
                _ = editor.load_history(HISTORY_FILE);

                loop {
                    let p = "(bs) ".to_string();
                    editor
                        .helper_mut()
                        .expect("unreachable: no helper")
                        .colored_prompt = format!("\x1b[1;32m{p}\x1b[0m");
                    let readline = editor.readline(&p);
                    match readline {
                        Ok(input) => {
                            if input == "q" || input == "quit" {
                                control_tx.send(Control::Terminate).unwrap();
                                break;
                            } else {
                                editor.add_history_entry(&input).unwrap();
                                control_tx.send(Control::Cmd(input)).unwrap();
                            }
                        }
                        Err(err) => {
                            let on_sign = |sign: Signal| {
                                if self.control_tx.try_send(Control::Terminate).is_err() {
                                    kill(*self.debugee_pid.lock().unwrap(), sign).unwrap();
                                    self.control_tx.send(Control::Terminate).unwrap();
                                }
                            };

                            match err {
                                ReadlineError::Eof | ReadlineError::Interrupted => {
                                    on_sign(Signal::SIGINT);
                                    break;
                                }
                                _ => {
                                    println!("error: {:#}", err);
                                    control_tx.send(Control::Terminate).unwrap();
                                    break;
                                }
                            }
                        }
                    }
                }

                editor.append_history(HISTORY_FILE).unwrap();
            });
        }

        let app_loop = AppLoop {
            debugger: self.debugger,
            control_rx: self.control_rx,
            printer: external_printer,
        };

        app_loop.run();

        Ok(())
    }
}

struct AppLoop {
    debugger: Debugger,
    control_rx: Receiver<Control>,
    printer: ExternalPrinter,
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
            Command::Continue => Continue::new(&mut self.debugger).handle()?,
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

                if ALREADY_RUN.load(Ordering::SeqCst) {
                    if self.yes("Restart program? (y or n)")? {
                        Run::new(&mut self.debugger).restart()?
                    }
                } else {
                    Run::new(&mut self.debugger).start()?;
                    ALREADY_RUN.store(true, Ordering::SeqCst);
                }
            }
            Command::StepInstruction => StepI::new(&mut self.debugger).handle()?,
            Command::StepInto => StepInto::new(&mut self.debugger).handle()?,
            Command::StepOut => StepOut::new(&mut self.debugger).handle()?,
            Command::StepOver => StepOver::new(&mut self.debugger).handle()?,
            Command::Breakpoint(bp_cmd) => Break::new(&mut self.debugger).handle(bp_cmd)?,
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
            Command::Help(reason) => match reason {
                None => {
                    self.printer.print("help here (TODO)");
                }
                Some(reason) => {
                    self.printer.print(reason);
                    self.printer.print("help here (TODO)");
                }
            },
            Command::PrintSymbol(symbol) => {
                let symbol = Symbol::new(&self.debugger).handle(&symbol)?;
                self.printer
                    .print(format!("{:?} {:#016X}", symbol.kind, symbol.addr));
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
                    self.printer.print(format!("> {}", command));
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
