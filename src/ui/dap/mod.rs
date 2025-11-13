mod hook;
mod logger;
mod server;
mod variable;

use std::io::{BufRead, BufReader, Stdout};
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};

use anyhow::anyhow;
use chumsky::Parser;
use dap::events::{Event, OutputEventBody};
use dap::requests::{Command, Request, VariablesArguments};
use dap::responses::{
    ContinueResponse, ResponseBody, ScopesResponse, SetBreakpointsResponse, StackTraceResponse,
    ThreadsResponse, VariablesResponse,
};
use dap::server::ServerOutput;
use dap::types::{
    Breakpoint, Capabilities, OutputEventCategory, Scope, ScopePresentationhint, Source,
    SourceBreakpoint, StackFrame, StackFramePresentationhint, Thread,
};
use itertools::Itertools;
use logger::DapLogger;
use nix::sys::signal::Signal::SIGKILL;

use super::supervisor;
use crate::debugger::variable::Identity;
use crate::debugger::variable::dqe::{Dqe, Selector};
use crate::debugger::variable::value::Value;
use crate::debugger::{Debugger, DebuggerBuilder, ThreadSnapshot};
use crate::ui::command::parser::expression::parser;
use crate::ui::dap::hook::DapHook;
use crate::ui::dap::server::DapServer;
use crate::ui::dap::variable::ReferenceRegistry;
use crate::ui::supervisor::DebugeeSource;

pub struct DapApplication {
    debugger_builder: Arc<dyn Fn() -> DebuggerBuilder<DapHook> + Send + Sync>,
    server: DapServer,
    session: Option<Session>,

    var_ref_registry: ReferenceRegistry,
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
            session: None,
            var_ref_registry: ReferenceRegistry::default(),
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
                        supports_single_thread_execution_requests: Some(false),
                        ..Default::default()
                    }),
                )?;
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

                    move || {
                        let result = debugger_thread(
                            program,
                            cwd,
                            debugger_builder,
                            output,
                            launched_sender,
                            command_receiver,
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

                self.server.send_event(Event::Initialized)?;
            }
            Command::SetBreakpoints(args) => {
                let Some(session) = &self.session else {
                    self.server.respond_error(req.seq, "No running session")?;
                    anyhow::bail!("No running session");
                };

                let breakpoints = args
                    .breakpoints
                    .iter()
                    .flatten()
                    .cloned()
                    .map(|bp| (args.source.clone(), bp))
                    .collect_vec();

                let (sender, receiver) = mpsc::sync_channel(0);

                session
                    .command_sender
                    .send(DebuggerCommand::SetBreakpoints(breakpoints, sender))?;

                let breakpoint_ids = receiver.recv()?;

                self.server.respond_success(
                    req.seq,
                    ResponseBody::SetBreakpoints(SetBreakpointsResponse {
                        breakpoints: args
                            .breakpoints
                            .iter()
                            .flatten()
                            .zip(breakpoint_ids)
                            .map(|(bp, id)| Breakpoint {
                                id,
                                source: Some(args.source.clone()),
                                line: Some(bp.line),
                                verified: id.is_some(),
                                ..Default::default()
                            })
                            .collect_vec(),
                    }),
                )?;
            }
            Command::ConfigurationDone => {
                let Some(session) = &self.session else {
                    self.server.respond_error(req.seq, "No running session")?;
                    anyhow::bail!("No running session");
                };

                session.command_sender.send(DebuggerCommand::Start)?;

                self.server
                    .respond_success(req.seq, ResponseBody::ConfigurationDone)?;
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
                                // TODO add frames/threads len checks
                                let id = FrameInfo {
                                    thread_id: thread.thread.number as u16,
                                    frame_id: idx as u16,
                                };

                                if let Some(place) = frame.place {
                                    StackFrame {
                                        id: id.pack() as i64,
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
                                        id: id.pack() as i64,
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
                                variables_reference: variable::VarRef {
                                    scope: variable::VarScope::Args,
                                    frame_info: args.frame_id as u32,
                                    var_id: 0,
                                }
                                .decode(), // Can't use 0 as reference
                                expensive: false,
                                ..Default::default()
                            },
                            Scope {
                                name: "Locals".to_owned(),
                                presentation_hint: Some(ScopePresentationhint::Locals),
                                variables_reference: variable::VarRef {
                                    scope: variable::VarScope::Locals,
                                    frame_info: args.frame_id as u32,
                                    var_id: 0,
                                }
                                .decode(),
                                expensive: false,
                                ..Default::default()
                            },
                        ],
                    }),
                )?;
            }
            Command::Variables(args) => {
                self.handle_var_request(args, req.seq)?;
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

    fn handle_var_request(&mut self, args: &VariablesArguments, seq: i64) -> anyhow::Result<()> {
        let Some(session) = &self.session else {
            return Ok(());
        };

        let var_ref = variable::VarRef::encode(args.variables_reference as u64);
        let frame_info = FrameInfo::unpack(var_ref.frame_info as i64);

        session
            .command_sender
            .send(DebuggerCommand::FocusThread(frame_info.thread_id as u32))?;

        session
            .command_sender
            .send(DebuggerCommand::FocusFrame(frame_info.frame_id as u32))?;

        let (sender, receiver) = mpsc::sync_channel(0);

        let variables = if var_ref.var_id == 0 {
            let cmd = if var_ref.scope == variable::VarScope::Args {
                DebuggerCommand::Args(Dqe::Variable(Selector::Any), sender)
            } else {
                DebuggerCommand::Locals(Dqe::Variable(Selector::Any), sender)
            };

            session.command_sender.send(cmd)?;
            let variables = receiver.recv()?;

            variables
                .into_iter()
                .filter_map(|(ident, val)| {
                    let path = ident.name.as_deref().unwrap_or("");
                    let name = ident.name.as_deref().unwrap_or("unknown");

                    variable::into_dap_var_repr(
                        &mut self.var_ref_registry,
                        var_ref,
                        path,
                        name,
                        &val,
                    )
                })
                .collect_vec()
        } else {
            let path = self.var_ref_registry.get_path(var_ref.var_id).to_string();

            let dqe = parser()
                .parse(&path)
                .into_result()
                .map_err(|_| anyhow!("parse request DQE error"))?;

            let cmd = if var_ref.scope == variable::VarScope::Args {
                DebuggerCommand::Args(dqe, sender)
            } else {
                DebuggerCommand::Locals(dqe, sender)
            };

            session.command_sender.send(cmd)?;
            let variables = receiver.recv()?;

            variables
                .into_iter()
                .map(|(_, val)| {
                    variable::expand_and_collect(&mut self.var_ref_registry, var_ref, &path, &val)
                })
                .flatten()
                .collect_vec()
        };

        self.server.respond_success(
            seq,
            ResponseBody::Variables(VariablesResponse { variables }),
        )?;

        Ok(())
    }
}

pub enum DebuggerCommand {
    Start,
    StepOver,
    StepIn,
    StepOut,
    Continue,
    Exit,
    FocusFrame(u32),
    FocusThread(u32),
    SetBreakpoints(
        Vec<(Source, SourceBreakpoint)>,
        mpsc::SyncSender<Vec<Option<i64>>>,
    ),
    Threads(mpsc::SyncSender<Vec<ThreadSnapshot>>),
    Args(Dqe, mpsc::SyncSender<Vec<(Identity, Value)>>),
    Locals(Dqe, mpsc::SyncSender<Vec<(Identity, Value)>>),
}

fn debugger_thread(
    program: String,
    cwd: Option<String>,
    debugger_builder: Arc<dyn Fn() -> DebuggerBuilder<DapHook>>,
    output: Arc<Mutex<ServerOutput<Stdout>>>,
    launched_sender: mpsc::SyncSender<nix::unistd::Pid>,
    command_receiver: mpsc::Receiver<DebuggerCommand>,
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

    while let Ok(command) = command_receiver.recv() {
        match handle_debugger_command(&mut debugger, command) {
            Ok(should_continue) => {
                if !should_continue {
                    break;
                }
            }
            Err(e) => {
                log::error!("{e}");
            }
        }
    }

    log::debug!("Debugger thread exiting");

    Ok(())
}

fn handle_debugger_command(
    debugger: &mut Debugger,
    command: DebuggerCommand,
) -> anyhow::Result<bool> {
    match command {
        DebuggerCommand::Start => {
            log::debug!("Starting execution");
            debugger.start_debugee()?;
        }
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
            return Ok(false);
        }
        DebuggerCommand::FocusFrame(n) => {
            let _ = debugger.set_frame_into_focus(n);
        }
        DebuggerCommand::FocusThread(n) => {
            let _ = debugger.set_thread_into_focus(n);
        }
        DebuggerCommand::Threads(sender) => {
            sender.send(debugger.thread_state()?)?;
        }
        DebuggerCommand::Args(dqe, sender) => {
            sender
                .send(
                    debugger
                        .read_argument(dqe)?
                        .into_iter()
                        .map(|v| v.into_identified_value())
                        .collect_vec(),
                )
                .map_err(|e| anyhow!("{e}"))?;
        }
        DebuggerCommand::Locals(dqe, sender) => {
            sender
                .send(
                    debugger
                        .read_variable(dqe)?
                        .into_iter()
                        .map(|v| v.into_identified_value())
                        .collect_vec(),
                )
                .map_err(|e| anyhow!("{e}"))?;
        }
        DebuggerCommand::SetBreakpoints(breakpoints, sender) => {
            let mut breakpoint_ids = Vec::new();

            for (source, bp) in breakpoints {
                let id = source.path.and_then(|path| {
                    let brkpts = debugger
                        .set_breakpoint_at_line(&path, bp.line as u64)
                        .inspect_err(|e| log::error!("breakpoint: {e}"))
                        .ok()?;

                    brkpts.first().map(|breakpoint| breakpoint.number as i64)
                });

                breakpoint_ids.push(id);
            }

            sender.send(breakpoint_ids)?;
        }
    }

    Ok(true)
}

#[derive(Clone, Copy, PartialEq)]
struct FrameInfo {
    thread_id: u16,
    frame_id: u16,
}

impl FrameInfo {
    pub fn pack(self) -> u32 {
        let packed = ((self.thread_id as u32) << 16) | self.frame_id as u32;

        packed
    }

    pub fn unpack(packed: i64) -> Self {
        let packed = packed as u64;
        Self {
            thread_id: (packed >> 16) as u16,
            frame_id: (packed & 0xFFFF) as u16,
        }
    }
}
