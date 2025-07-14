mod logger;

use std::io::{self, BufRead, BufReader, BufWriter, Stdin, Stdout};
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};

use anyhow::anyhow;
use dap::events::{Event, ExitedEventBody, OutputEventBody, StoppedEventBody};
use dap::requests::{Command, LaunchRequestArguments, Request};
use dap::responses::{
    ContinueResponse, ResponseBody, ScopesResponse, SetBreakpointsResponse, StackTraceResponse,
    ThreadsResponse, VariablesResponse,
};
use dap::server::{Server, ServerOutput};
use dap::types::{
    Breakpoint, Capabilities, OutputEventCategory, Scope, ScopePresentationhint, Source,
    SourceBreakpoint, StackFrame, StackFramePresentationhint, StoppedEventReason, Thread, Variable,
};
use itertools::Itertools;
use logger::DapLogger;
use nix::sys::signal::Signal::SIGKILL;

use crate::debugger::variable::Identity;
use crate::debugger::variable::dqe::{Dqe, Selector};
use crate::debugger::variable::value::Value;
use crate::debugger::{DebuggerBuilder, EventHook, ThreadSnapshot};
use crate::ui::supervisor::DebugeeSource;

use super::supervisor;

pub struct DapApplication {
    debugger_builder: Arc<dyn Fn() -> DebuggerBuilder<DapHook> + Send + Sync>,
    server: Server<Stdin, Stdout>,
    is_config_done: bool,
    buffered_launch_request: Option<(i64, LaunchRequestArguments)>,
    breakpoints: Vec<(Source, SourceBreakpoint)>,
    session: Option<Session>,
}

struct Session {
    pid: nix::unistd::Pid,
    command_sender: mpsc::SyncSender<DebuggerCommand>,
}

impl Session {
    fn request<T>(
        &self,
        cmd: impl Fn(mpsc::SyncSender<T>) -> DebuggerCommand,
    ) -> anyhow::Result<T> {
        let (sender, receiver) = mpsc::sync_channel(0);
        self.command_sender.send(cmd(sender))?;
        let result = receiver.recv()?;
        Ok(result)
    }
}

impl DapApplication {
    pub fn new(
        debugger_builder: impl Fn() -> DebuggerBuilder<DapHook> + Send + Sync + 'static,
    ) -> anyhow::Result<DapApplication> {
        let input = BufReader::new(io::stdin());
        let output = BufWriter::new(io::stdout());

        let server = Server::new(input, output);

        Ok(DapApplication {
            debugger_builder: Arc::new(debugger_builder),
            server,
            is_config_done: false,
            buffered_launch_request: None,
            breakpoints: vec![],
            session: None,
        })
    }

    pub fn run(mut self) -> anyhow::Result<supervisor::ControlFlow> {
        let logger = DapLogger::new(self.server.output.clone());
        let filter = logger.filter();
        crate::log::LOGGER_SWITCHER.switch(logger, filter);

        loop {
            let req = match self.server.poll_request() {
                Ok(Some(req)) => req,
                Ok(None) => continue,
                Err(e) => {
                    log::error!("{e}");
                    continue;
                }
            };

            // Vscode sends breakpoint configuration concurrently with the launch request for some reason. To make sure
            // that we set breakpoints *before* starting the debuggee, defer processing the launch request until we
            // receive a ConfigurationDone.
            if !self.is_config_done {
                if let Command::Launch(args) = req.command {
                    self.buffered_launch_request = Some((req.seq, args));
                    continue;
                }
            }

            log::debug!("{}: {:?}", req.seq, req.command);

            match self.handle_request(req) {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => {
                    log::error!("{e}")
                }
            }

            if self.is_config_done {
                if let Some((seq, args)) = self.buffered_launch_request.take() {
                    let req = Request {
                        seq,
                        command: Command::Launch(args),
                    };

                    log::debug!("{}: {:?}", req.seq, req.command);

                    match self.handle_request(req) {
                        Ok(true) => {}
                        Ok(false) => break,
                        Err(e) => {
                            log::error!("{e}")
                        }
                    }
                }
            }
        }

        Ok(supervisor::ControlFlow::Exit)
    }

    fn handle_request(&mut self, req: Request) -> anyhow::Result<bool> {
        match &req.command {
            Command::Initialize(_args) => {
                self.server
                    .respond(req.success(ResponseBody::Initialize(Capabilities {
                        supports_configuration_done_request: Some(true),
                        ..Default::default()
                    })))?;

                self.server.send_event(Event::Initialized)?;
            }
            Command::SetBreakpoints(args) => {
                if self.session.is_none() {
                    self.breakpoints.extend(
                        args.breakpoints
                            .iter()
                            .flatten()
                            .cloned()
                            .map(|bp| (args.source.clone(), bp)),
                    );

                    self.server.respond(
                        req.success(ResponseBody::SetBreakpoints(SetBreakpointsResponse {
                            breakpoints: self
                                .breakpoints
                                .iter()
                                .map(|(source, bp)| Breakpoint {
                                    source: Some(source.clone()),
                                    line: Some(bp.line),
                                    id: Some(1),
                                    verified: true,
                                    ..Default::default()
                                })
                                .collect_vec(),
                        })),
                    )?;
                } else {
                    self.server
                        .respond(req.error("Can't update breakpoints while program is running"))?;
                }
            }
            Command::ConfigurationDone => {
                self.is_config_done = true;
                self.server
                    .respond(req.success(ResponseBody::ConfigurationDone))?;
            }
            Command::Launch(args) => {
                let data = args
                    .additional_data
                    .as_ref()
                    .ok_or_else(|| anyhow!("missing launch arguments"))?;

                let program = data.get("program").unwrap().as_str().unwrap().to_owned();
                let cwd = data
                    .get("cwd")
                    .and_then(|cwd| cwd.as_str())
                    .map(ToOwned::to_owned);

                let (command_sender, command_receiver) = mpsc::sync_channel(0);
                let (launched_sender, launched_receiver) = mpsc::sync_channel(0);

                std::thread::spawn({
                    let debugger_builder: Arc<dyn Fn() -> DebuggerBuilder<DapHook> + Send + Sync> =
                        self.debugger_builder.clone();
                    let output = self.server.output.clone();
                    let breakpoints = std::mem::take(&mut self.breakpoints);

                    move || {
                        let result = debugger_thread(
                            program,
                            cwd,
                            debugger_builder,
                            output,
                            launched_sender,
                            command_receiver,
                            breakpoints,
                        );

                        if let Err(e) = result {
                            log::error!("{e}");
                        }
                    }
                });

                let pid = launched_receiver.recv().unwrap();

                self.session = Some(Session {
                    pid,
                    command_sender,
                });

                log::info!("launch successful");

                self.server.respond(req.success(ResponseBody::Launch))?;
            }
            Command::Threads => {
                if let Some(session) = &self.session {
                    let threads = session.request(DebuggerCommand::Threads)?;
                    self.server.respond(
                        req.success(ResponseBody::Threads(ThreadsResponse {
                            threads: threads
                                .iter()
                                .map(|thread| Thread {
                                    id: thread.thread.number.into(),
                                    name: format!("Thread #{}", thread.thread.number),
                                })
                                .collect_vec(),
                        })),
                    )?;
                }
            }
            Command::StackTrace(args) => {
                if let Some(session) = &self.session {
                    let threads = session.request(DebuggerCommand::Threads)?;
                    let thread = threads
                        .into_iter()
                        .find(|thread| i64::from(thread.thread.number) == args.thread_id);
                    if let Some(thread) = thread {
                        let stack_frames = thread
                            .bt
                            .into_iter()
                            .flatten()
                            .enumerate()
                            .map(|(idx, frame)| {
                                if let Some(place) = frame.place {
                                    StackFrame {
                                        id: idx as i64,
                                        name: frame
                                            .func_name
                                            .unwrap_or_else(|| "Unknown".to_owned()),
                                        source: Some(Source {
                                            path: Some(place.file.to_string_lossy().into_owned()),
                                            ..Default::default()
                                        }),
                                        line: place.line_number.try_into().unwrap(),
                                        column: place.column_number.try_into().unwrap(),
                                        ..Default::default()
                                    }
                                } else {
                                    StackFrame {
                                        id: idx as i64,
                                        name: "Unknown".to_owned(),
                                        presentation_hint: Some(StackFramePresentationhint::Subtle),
                                        ..Default::default()
                                    }
                                }
                            })
                            .collect_vec();

                        self.server.respond(req.success(ResponseBody::StackTrace(
                            StackTraceResponse {
                                total_frames: Some(stack_frames.len().try_into().unwrap()),
                                stack_frames,
                            },
                        )))?;
                    } else {
                        self.server.respond(req.error("Thread not found"))?;
                    }
                }
            }
            Command::Scopes(_args) => {
                // TODO: Check which frame was requested
                self.server
                    .respond(req.success(ResponseBody::Scopes(ScopesResponse {
                        scopes: vec![Scope {
                            name: "Locals".to_owned(),
                            presentation_hint: Some(ScopePresentationhint::Locals),
                            variables_reference: 1,
                            expensive: false,
                            ..Default::default()
                        }],
                    })))?;
            }
            Command::Variables(_args) => {
                // TODO: Check which scope/frame was requested
                if let Some(session) = &self.session {
                    let variables = session
                        .request(DebuggerCommand::Variables)?
                        .into_iter()
                        .map(|(identity, value)| Variable {
                            name: identity.name.unwrap_or_else(|| "Unknown".to_string()),
                            value: format!("{value:?}"),
                            ..Default::default()
                        })
                        .collect_vec();

                    self.server.respond(
                        req.success(ResponseBody::Variables(VariablesResponse { variables })),
                    )?;
                }
            }
            Command::Next(_args) => {
                if let Some(session) = &self.session {
                    session.command_sender.send(DebuggerCommand::StepOver)?;
                    self.server.respond(req.success(ResponseBody::Next))?;
                }
            }
            Command::StepIn(_args) => {
                if let Some(session) = &self.session {
                    session.command_sender.send(DebuggerCommand::StepIn)?;
                    self.server.respond(req.success(ResponseBody::StepIn))?;
                }
            }
            Command::StepOut(_args) => {
                if let Some(session) = &self.session {
                    session.command_sender.send(DebuggerCommand::StepOut)?;
                    self.server.respond(req.success(ResponseBody::StepOut))?;
                }
            }
            Command::Continue(_args) => {
                if let Some(session) = &self.session {
                    session.command_sender.send(DebuggerCommand::Continue)?;
                    self.server
                        .respond(req.success(ResponseBody::Continue(ContinueResponse {
                            ..Default::default()
                        })))?;
                }
            }
            Command::Disconnect(_) => {
                if let Some(session) = self.session.take() {
                    let _ = nix::sys::signal::kill(session.pid, SIGKILL)
                        .inspect_err(|e| log::error!("{e}"));
                    session.command_sender.send(DebuggerCommand::Exit)?;
                } else {
                    log::warn!("no active debug session");
                }
                return Ok(false);
            }
            _ => {
                log::warn!("unknown command: {:?}", req.command);
                self.server.respond(req.cancellation())?;
            }
        }

        Ok(true)
    }
}

pub struct DapHook {
    output: Arc<Mutex<ServerOutput<Stdout>>>,
}

impl EventHook for DapHook {
    fn on_breakpoint(
        &self,
        pc: crate::debugger::address::RelocatedAddress,
        num: u32,
        place: Option<crate::debugger::PlaceDescriptor>,
        function: Option<&crate::debugger::FunctionDie>,
    ) -> anyhow::Result<()> {
        let mut output = self.output.lock().unwrap();

        output
            .send_event(Event::Stopped(StoppedEventBody {
                reason: StoppedEventReason::Breakpoint,
                description: None,
                thread_id: Some(1),
                preserve_focus_hint: None,
                text: None,
                all_threads_stopped: None,
                hit_breakpoint_ids: Some(vec![1]),
            }))
            .unwrap();

        Ok(())
    }

    fn on_watchpoint(
        &self,
        pc: crate::debugger::address::RelocatedAddress,
        num: u32,
        place: Option<crate::debugger::PlaceDescriptor>,
        condition: crate::debugger::register::debug::BreakCondition,
        dqe_string: Option<&str>,
        old_value: Option<&crate::debugger::variable::value::Value>,
        new_value: Option<&crate::debugger::variable::value::Value>,
        end_of_scope: bool,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn on_step(
        &self,
        pc: crate::debugger::address::RelocatedAddress,
        place: Option<crate::debugger::PlaceDescriptor>,
        function: Option<&crate::debugger::FunctionDie>,
    ) -> anyhow::Result<()> {
        let mut output = self.output.lock().unwrap();

        output
            .send_event(Event::Stopped(StoppedEventBody {
                reason: StoppedEventReason::Step,
                description: None,
                thread_id: Some(1),
                preserve_focus_hint: None,
                text: None,
                all_threads_stopped: None,
                hit_breakpoint_ids: None,
            }))
            .unwrap();

        Ok(())
    }

    fn on_async_step(
        &self,
        pc: crate::debugger::address::RelocatedAddress,
        place: Option<crate::debugger::PlaceDescriptor>,
        function: Option<&crate::debugger::FunctionDie>,
        task_id: u64,
        task_completed: bool,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn on_signal(&self, signal: nix::sys::signal::Signal) {}

    fn on_exit(&self, code: i32) {
        let mut output = self.output.lock().unwrap();

        output.send_event(Event::Terminated(None)).unwrap();

        output
            .send_event(Event::Exited(ExitedEventBody {
                exit_code: code.into(),
            }))
            .unwrap();
    }

    fn on_process_install(&self, pid: thread_db::Pid, object: Option<&object::File>) {}
}

enum DebuggerCommand {
    StepOver,
    StepIn,
    StepOut,
    Continue,
    Exit,
    Threads(mpsc::SyncSender<Vec<ThreadSnapshot>>),
    Variables(mpsc::SyncSender<Vec<(Identity, Value)>>),
}

fn debugger_thread(
    program: String,
    cwd: Option<String>,
    debugger_builder: Arc<dyn Fn() -> DebuggerBuilder<DapHook>>,
    output: Arc<Mutex<ServerOutput<Stdout>>>,
    launched_sender: mpsc::SyncSender<nix::unistd::Pid>,
    command_receiver: mpsc::Receiver<DebuggerCommand>,
    breakpoints: Vec<(Source, SourceBreakpoint)>,
) -> anyhow::Result<()> {
    let source = DebugeeSource::File {
        path: &program,
        args: &[],
        cwd: cwd.as_deref().map(Path::new),
    };

    let (stdout_reader, stdout_writer) = os_pipe::pipe()?;
    let (stderr_reader, stderr_writer) = os_pipe::pipe()?;

    let process = source.create_child(stdout_writer, stderr_writer)?;
    let pid = process.pid();

    let mut debugger = (debugger_builder)()
        .with_hooks(DapHook {
            output: output.clone(),
        })
        .build(process)?;

    for (reader, category) in [
        (stdout_reader, OutputEventCategory::Stdout),
        (stderr_reader, OutputEventCategory::Stderr),
    ] {
        std::thread::spawn({
            let output = output.clone();
            move || {
                let mut stream = BufReader::new(reader);
                loop {
                    let mut line = String::new();
                    let Ok(size) = stream.read_line(&mut line) else {
                        break;
                    };

                    if size == 0 {
                        break;
                    }

                    output
                        .lock()
                        .unwrap()
                        .send_event(Event::Output(OutputEventBody {
                            category: Some(category.clone()),
                            output: line,
                            ..Default::default()
                        }))
                        .unwrap();
                }
            }
        });
    }

    launched_sender.send(pid)?;

    for (source, bp) in breakpoints {
        if let Some(path) = source.path {
            debugger.set_breakpoint_at_line(&path, bp.line.try_into()?)?;
        }
    }

    debugger.start_debugee()?;

    while let Ok(command) = command_receiver.recv() {
        match command {
            DebuggerCommand::StepOver => {
                debugger.step_over()?;
            }
            DebuggerCommand::StepIn => {
                debugger.step_into()?;
            }
            DebuggerCommand::StepOut => {
                debugger.step_out()?;
            }
            DebuggerCommand::Continue => {
                debugger.continue_debugee()?;
            }
            DebuggerCommand::Exit => {
                break;
            }
            DebuggerCommand::Threads(sender) => {
                sender.send(debugger.thread_state()?)?;
            }
            DebuggerCommand::Variables(sender) => {
                sender
                    .send(
                        debugger
                            .read_argument(Dqe::Variable(Selector::Any))?
                            .into_iter()
                            .chain(debugger.read_local_variables()?)
                            .map(|v| v.into_identified_value())
                            .collect_vec(),
                    )
                    .map_err(|e| anyhow!("{e}"))?;
            }
        }
    }

    log::debug!("Debugger thread exiting");

    Ok(())
}
