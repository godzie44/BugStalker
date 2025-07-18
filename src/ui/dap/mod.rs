mod hook;
mod logger;
mod server;

use std::io::{BufRead, BufReader, Stdout};
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};

use anyhow::anyhow;
use dap::events::{Event, OutputEventBody};
use dap::requests::{Command, Request};
use dap::responses::{
    ContinueResponse, ResponseBody, ScopesResponse, SetBreakpointsResponse, StackTraceResponse,
    ThreadsResponse, VariablesResponse,
};
use dap::server::ServerOutput;
use dap::types::{
    Breakpoint, Capabilities, OutputEventCategory, Scope, ScopePresentationhint, Source,
    SourceBreakpoint, StackFrame, StackFramePresentationhint, Thread, Variable,
};
use itertools::Itertools;
use logger::DapLogger;
use nix::sys::signal::Signal::SIGKILL;

use crate::debugger::variable::Identity;
use crate::debugger::variable::dqe::{Dqe, Selector};
use crate::debugger::variable::value::Value;
use crate::debugger::{DebuggerBuilder, ThreadSnapshot};
use crate::ui::dap::hook::DapHook;
use crate::ui::dap::server::DapServer;
use crate::ui::supervisor::DebugeeSource;

use super::supervisor;

pub struct DapApplication {
    debugger_builder: Arc<dyn Fn() -> DebuggerBuilder<DapHook> + Send + Sync>,
    server: DapServer,
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
        Ok(DapApplication {
            debugger_builder: Arc::new(debugger_builder),
            server: DapServer::new(),
            breakpoints: vec![],
            session: None,
        })
    }

    pub fn run(mut self) -> anyhow::Result<supervisor::ControlFlow> {
        let logger = DapLogger::new(self.server.output());
        let filter = logger.filter();
        crate::log::LOGGER_SWITCHER.switch(logger, filter);

        loop {
            let req = match self.server.poll_request() {
                Ok(Some(req)) => req,
                Ok(None) => {
                    log::warn!("Unexpected end of input stream");
                    break;
                }
                Err(e) => {
                    log::error!("{e}");
                    continue;
                }
            };

            match self.handle_request(req) {
                Ok(true) => { /* Success */ }
                Ok(false) => break,
                Err(e) => {
                    log::error!("{e}");
                }
            }
        }

        Ok(supervisor::ControlFlow::Exit)
    }

    fn handle_request(&mut self, req: Request) -> anyhow::Result<bool> {
        match &req.command {
            Command::Initialize(_args) => {
                self.server.respond_success(
                    req.seq,
                    ResponseBody::Initialize(Capabilities {
                        supports_configuration_done_request: Some(true),
                        ..Default::default()
                    }),
                )?;

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

                    self.server.respond_success(
                        req.seq,
                        ResponseBody::SetBreakpoints(SetBreakpointsResponse {
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
                        }),
                    )?;
                } else {
                    self.server.respond_error(
                        req.seq,
                        "Can't update breakpoints while program is running",
                    )?;
                }
            }
            Command::ConfigurationDone => {
                self.server
                    .respond_success(req.seq, ResponseBody::ConfigurationDone)?;
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
                    let output = self.server.output();
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

                self.server.respond_success(req.seq, ResponseBody::Launch)?;
            }
            Command::Threads => {
                if let Some(session) = &self.session {
                    let threads = session.request(DebuggerCommand::Threads)?;
                    self.server.respond_success(
                        req.seq,
                        ResponseBody::Threads(ThreadsResponse {
                            threads: threads
                                .iter()
                                .map(|thread| Thread {
                                    id: thread.thread.number.into(),
                                    name: format!("Thread #{}", thread.thread.number),
                                })
                                .collect_vec(),
                        }),
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

                        self.server.respond_success(
                            req.seq,
                            ResponseBody::StackTrace(StackTraceResponse {
                                total_frames: Some(stack_frames.len().try_into().unwrap()),
                                stack_frames,
                            }),
                        )?;
                    } else {
                        self.server.respond_error(req.seq, "Thread not found")?;
                    }
                }
            }
            Command::Scopes(args) => {
                self.server.respond_success(
                    req.seq,
                    ResponseBody::Scopes(ScopesResponse {
                        scopes: vec![
                            Scope {
                                name: "Args".to_owned(),
                                presentation_hint: Some(ScopePresentationhint::Arguments),
                                variables_reference: args.frame_id * 2 + 1, // Can't use 0 as reference
                                expensive: false,
                                ..Default::default()
                            },
                            Scope {
                                name: "Locals".to_owned(),
                                presentation_hint: Some(ScopePresentationhint::Locals),
                                variables_reference: args.frame_id * 2 + 2, // Can't use 0 as reference
                                expensive: false,
                                ..Default::default()
                            },
                        ],
                    }),
                )?;
            }
            Command::Variables(args) => {
                if let Some(session) = &self.session {
                    let frame_num = (args.variables_reference - 1) / 2;

                    session
                        .command_sender
                        .send(DebuggerCommand::FocusFrame(frame_num.try_into()?))?;

                    let variables = session
                        .request(if args.variables_reference % 2 == 1 {
                            DebuggerCommand::Args
                        } else {
                            DebuggerCommand::Locals
                        })?
                        .into_iter()
                        .map(|(identity, value)| Variable {
                            name: identity.name.unwrap_or_else(|| "Unknown".to_string()),
                            value: format!("{value:?}"),
                            ..Default::default()
                        })
                        .collect_vec();

                    self.server.respond_success(
                        req.seq,
                        ResponseBody::Variables(VariablesResponse { variables }),
                    )?;
                }
            }
            Command::Next(_args) => {
                if let Some(session) = &self.session {
                    session.command_sender.send(DebuggerCommand::StepOver)?;
                    self.server.respond_success(req.seq, ResponseBody::Next)?;
                }
            }
            Command::StepIn(_args) => {
                if let Some(session) = &self.session {
                    session.command_sender.send(DebuggerCommand::StepIn)?;
                    self.server.respond_success(req.seq, ResponseBody::StepIn)?;
                }
            }
            Command::StepOut(_args) => {
                if let Some(session) = &self.session {
                    session.command_sender.send(DebuggerCommand::StepOut)?;
                    self.server
                        .respond_success(req.seq, ResponseBody::StepOut)?;
                }
            }
            Command::Continue(_args) => {
                if let Some(session) = &self.session {
                    session.command_sender.send(DebuggerCommand::Continue)?;
                    self.server.respond_success(
                        req.seq,
                        ResponseBody::Continue(ContinueResponse {
                            ..Default::default()
                        }),
                    )?;
                }
            }
            Command::Disconnect(_) => {
                if let Some(session) = self.session.take() {
                    let _ = nix::sys::signal::kill(session.pid, SIGKILL)
                        .inspect_err(|e| log::error!("{e}"));
                    session.command_sender.send(DebuggerCommand::Exit)?;
                    self.server
                        .respond_success(req.seq, ResponseBody::Disconnect)?;
                } else {
                    log::warn!("No active debug session");
                    self.server
                        .respond_error(req.seq, "No active debug session")?;
                }
                return Ok(false);
            }
            _ => {
                log::warn!("unknown command: {:?}", req.command);
                self.server.respond_cancel(req.seq)?;
            }
        }

        Ok(true)
    }
}

enum DebuggerCommand {
    StepOver,
    StepIn,
    StepOut,
    Continue,
    Exit,
    FocusFrame(u32),
    Threads(mpsc::SyncSender<Vec<ThreadSnapshot>>),
    Args(mpsc::SyncSender<Vec<(Identity, Value)>>),
    Locals(mpsc::SyncSender<Vec<(Identity, Value)>>),
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
        .with_hooks(DapHook::new(output.clone()))
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
            DebuggerCommand::FocusFrame(n) => {
                let _ = debugger.set_frame_into_focus(n);
            }
            DebuggerCommand::Threads(sender) => {
                sender.send(debugger.thread_state()?)?;
            }
            DebuggerCommand::Args(sender) => {
                sender
                    .send(
                        debugger
                            .read_argument(Dqe::Variable(Selector::Any))?
                            .into_iter()
                            .map(|v| v.into_identified_value())
                            .collect_vec(),
                    )
                    .map_err(|e| anyhow!("{e}"))?;
            }
            DebuggerCommand::Locals(sender) => {
                sender
                    .send(
                        debugger
                            .read_variable(Dqe::Variable(Selector::Any))?
                            .into_iter()
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
