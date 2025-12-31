use super::command::r#async::Command as AsyncCommand;
use super::generic::trigger::TriggerRegistry;
use crate::debugger::process::{Child, Installed};
use crate::debugger::variable::dqe::{Dqe, Selector};
use crate::debugger::{Debugger, DebuggerBuilder};
use crate::ui::command::Command;
use crate::ui::command::CommandError;
use crate::ui::console::editor::{BSEditor, CommandCompleter};
use crate::ui::console::hook::TerminalHook;
use crate::ui::generic::command_handler::{CommandHandler, Completer, ProgramTaker, YesQuestion};
use crate::ui::generic::file::FileView;
use crate::ui::generic::help::*;
use crate::ui::generic::print;
use crate::ui::generic::print::ExternalPrinter;
use crate::ui::generic::print::style::ErrorView;
use crate::ui::generic::print::style::ImportantView;
use crate::ui::generic::trigger::UserProgram;
use crate::ui::supervisor;
use crate::ui::{DebugeeOutReader, config};
use crossterm::style::{Color, Stylize};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use rustyline::error::ReadlineError;
use std::io::{BufRead, BufReader};
use std::process::exit;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex, Once, mpsc};
use std::time::Duration;
use std::{thread, vec};
use timeout_readwrite::TimeoutReader;

pub mod cfg;
pub mod editor;
pub mod hook;

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
            ExternalPrinter::new_for_editor(&mut editor)?,
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
            let stdout_printer = ExternalPrinter::new_for_editor(&mut self.editor)?;
            thread::spawn(move || print_out!(stdout, "{}", stdout_printer, cancel1));

            let stderr = TimeoutReader::new(self.debugee_err.clone(), Duration::from_millis(1));
            let stderr_printer = ExternalPrinter::new_for_editor(&mut self.editor)?;
            thread::spawn(move || print_out!(stderr, "\x1b[31m{}", stderr_printer, cancel2));
        };

        let (ready_to_next_command_tx, ready_to_next_command_rx) = mpsc::channel();

        let helper = Helper::new(&self.debugger);
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
            printer: ExternalPrinter::new_for_editor(&mut self.editor)?,
            debugee_out: self.debugee_out.clone(),
            debugee_err: self.debugee_err.clone(),
            cancel_output_flag: cancel,
            ready_to_next_command_tx,
            helper,
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

        let error_printer = ExternalPrinter::new_for_editor(&mut self.editor)?;
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
    fn run(mut self) -> anyhow::Result<supervisor::ControlFlow> {
        struct ConsoleYes<'a> {
            user_input_rx: &'a Receiver<UserAction>,
            printer: &'a ExternalPrinter,
            ready_to_next_command_tx: mpsc::Sender<EditorMode>,
        }

        impl YesQuestion for ConsoleYes<'_> {
            fn yes(&self, question: &str) -> Result<bool, CommandError> {
                self.printer.println(question);

                loop {
                    _ = self.ready_to_next_command_tx.send(EditorMode::YesNo);
                    let act = self
                        .user_input_rx
                        .recv()
                        .expect("unexpected sender disconnect");
                    return match act {
                        UserAction::Cmd(cmd) => match cmd.to_lowercase().as_str() {
                            "y" | "yes" => Ok(true),
                            "n" | "no" => Ok(false),
                            _ => continue,
                        },
                        UserAction::Terminate | UserAction::ChangeMode | UserAction::Nop => {
                            Ok(false)
                        }
                    };
                }
            }
        }

        let yes = ConsoleYes {
            user_input_rx: &self.user_input_rx,
            printer: &self.printer,
            ready_to_next_command_tx: self.ready_to_next_command_tx.clone(),
        };

        struct ConsoleCompleter {
            completer: Arc<Mutex<CommandCompleter>>,
        }

        impl Completer for ConsoleCompleter {
            fn update_completer_variables(&self, debugger: &Debugger) -> anyhow::Result<()> {
                let vars = debugger.read_variable_names(Dqe::Variable(Selector::Any))?;
                let args = debugger.read_argument_names(Dqe::Variable(Selector::Any))?;

                let mut completer = self.completer.lock().unwrap();
                completer.replace_local_var_hints(vars);
                completer.replace_arg_hints(args);
                Ok(())
            }
        }

        let completer = ConsoleCompleter {
            completer: self.completer.clone(),
        };

        struct ConsoleProgramTaker<'a> {
            user_input_rx: &'a Receiver<UserAction>,
            printer: &'a ExternalPrinter,
            ready_to_next_command_tx: mpsc::Sender<EditorMode>,
        }

        impl ProgramTaker for ConsoleProgramTaker<'_> {
            fn take_user_command_list(&self, help: &str) -> Result<UserProgram, CommandError> {
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
        }

        let prog_taker = ConsoleProgramTaker {
            user_input_rx: &self.user_input_rx,
            printer: &self.printer,
            ready_to_next_command_tx: self.ready_to_next_command_tx.clone(),
        };

        let mut handler = CommandHandler {
            yes_handler: yes,
            complete_handler: completer,
            prog_taker,
            trigger_reg: &self.trigger_reg,
            debugger: &mut self.debugger,
            printer: &self.printer,
            file_view: &self.file_view,
            helper: &self.helper,
        };

        loop {
            if let Some(user_program) = self.trigger_reg.take_program() {
                self.printer
                    .println(ImportantView::from("Related program found:"));
                user_program.into_iter().for_each(|(cmd, _)| {
                    if let Err(e) = handler.handle_command(cmd) {
                        Self::handle_error(&self.printer, e);
                    }
                });
            };

            _ = self.ready_to_next_command_tx.send(EditorMode::Default);

            let Ok(action) = self.user_input_rx.recv() else {
                return Ok(supervisor::ControlFlow::Exit);
            };

            match action {
                UserAction::Cmd(command) => {
                    if !command.is_empty()
                        && let Err(e) =
                            Command::parse(&command).and_then(|cmd| handler.handle_command(cmd))
                    {
                        Self::handle_error(&self.printer, e);
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

    fn handle_error(printer: &ExternalPrinter, error: CommandError) {
        match error {
            CommandError::Parsing(pretty_error) => {
                printer.println(pretty_error);
            }
            CommandError::FileRender(_) => {
                printer.println(ErrorView::from(format!("Render file error: {error:#}")));
            }
            CommandError::Handle(ref err) if err.is_fatal() => {
                printer.println(ErrorView::from("Shutdown debugger"));
                printer.println(ErrorView::from(format!("Fatal error: {error:#}")));
                exit(1);
            }
            CommandError::Handle(_) => {
                printer.println(ErrorView::from(format!("Error: {error:#}")));
            }
        }
    }
}
