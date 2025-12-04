mod hook;
mod logger;
mod server;
mod variable;

use std::io::{BufRead, BufReader, Stdout};
use std::path::Path;
use std::sync::{Arc, Mutex};

use super::supervisor;
use crate::debugger::DebuggerBuilder;
use crate::debugger::variable::Identity;
use crate::debugger::variable::dqe::{Dqe, Selector};
use crate::debugger::variable::value::Value;
use crate::ui::command::parser::expression::parser;
use crate::ui::dap::hook::DapHook;
use crate::ui::dap::server::DapServer;
use crate::ui::dap::variable::ReferenceRegistry;
use crate::ui::proto::{self, ClientExchanger, ServerExchanger, exchanger};
use crate::ui::supervisor::DebugeeSource;
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
    StackFrame, StackFramePresentationhint, Thread,
};
use itertools::Itertools;
use logger::DapLogger;
use serde::Deserialize;

pub struct DapApplication {
    debugger_builder: Arc<dyn Fn() -> DebuggerBuilder<DapHook> + Send + Sync>,
    server: DapServer,
    session: Option<Session>,

    var_ref_registry: ReferenceRegistry,
}

struct Session {
    debugger_client: ClientExchanger,
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
        macro_rules! session_or_fail {
            () => {{
                let Some(session) = &self.session else {
                    self.server.respond_error(req.seq, "No running session")?;
                    anyhow::bail!("No running session");
                };
                session
            }};
        }

        match req.command {
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
            Command::Attach(args) => {
                #[derive(Deserialize, Debug)]
                #[serde(rename_all = "camelCase")]
                struct CustomAttachArgs {
                    #[serde(alias = "pid")]
                    process_id: u32,
                }

                let custom_args: CustomAttachArgs = serde_json::from_value(
                    args.additional_data
                        .ok_or(anyhow!("Additional data not found"))?,
                )?;
                let pid = custom_args.process_id;

                log::info!("attach to: {pid}");

                anyhow::bail!("Attach to process is not done yet");
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

                let ready_barrier = Arc::new(std::sync::Barrier::new(2));

                let (srv, client) = exchanger();

                std::thread::spawn({
                    let barrier = ready_barrier.clone();
                    let debugger_builder = self.debugger_builder.clone();
                    let output = self.server.output();

                    move || {
                        let result =
                            debugger_thread(program, cwd, debugger_builder, output, &barrier, srv);

                        if let Err(e) = result {
                            log::error!("{e}");
                        }
                    }
                });

                ready_barrier.wait();

                self.session = Some(Session {
                    debugger_client: client,
                });

                log::info!("launch successful");

                self.server.respond_success(req.seq, ResponseBody::Launch)?;

                self.server.send_event(Event::Initialized)?;
            }
            Command::SetBreakpoints(args) => {
                let session = session_or_fail!();

                let breakpoints = args
                    .breakpoints
                    .iter()
                    .flatten()
                    .cloned()
                    .map(|bp| (args.source.clone(), bp))
                    .collect_vec();

                let breakpoint_ids = session.debugger_client.request_sync(|debugger| {
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

                    breakpoint_ids
                })?;

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
                let session = session_or_fail!();

                session.debugger_client.request_sync(|debugger| {
                    log::debug!("Starting execution");
                    debugger.start_debugee()
                })??;

                self.server
                    .respond_success(req.seq, ResponseBody::ConfigurationDone)?;
            }
            Command::Threads => {
                let session = session_or_fail!();

                let threads = session
                    .debugger_client
                    .request_sync(|debugger| debugger.thread_state())??;

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
            Command::StackTrace(args) => {
                let session = session_or_fail!();

                let threads = session
                    .debugger_client
                    .request_sync(|debugger| debugger.thread_state())??;

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
                                    name: frame.func_name.unwrap_or_else(|| "Unknown".to_owned()),
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
                self.handle_var_request(&args, req.seq)?;
            }
            Command::Next(_args) => {
                let session = session_or_fail!();

                session
                    .debugger_client
                    .request_sync(|debugger| debugger.step_over())??;

                self.server.respond_success(req.seq, ResponseBody::Next)?;
            }
            Command::StepIn(_args) => {
                let session = session_or_fail!();

                session
                    .debugger_client
                    .request_sync(|debugger| debugger.step_into())??;

                self.server.respond_success(req.seq, ResponseBody::StepIn)?;
            }
            Command::StepOut(_args) => {
                let session = session_or_fail!();

                session
                    .debugger_client
                    .request_sync(|debugger| debugger.step_out())??;

                self.server
                    .respond_success(req.seq, ResponseBody::StepOut)?;
            }
            Command::Continue(_args) => {
                let session = session_or_fail!();

                session
                    .debugger_client
                    .request_sync(|debugger| debugger.continue_debugee())??;

                self.server.respond_success(
                    req.seq,
                    ResponseBody::Continue(ContinueResponse {
                        ..Default::default()
                    }),
                )?;
            }
            Command::Disconnect(_) => {
                if let Some(session) = self.session.take() {
                    session.debugger_client.send_exit_sync();

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

        let thread_id = frame_info.thread_id;
        session
            .debugger_client
            .request_sync(move |debugger| debugger.set_thread_into_focus(thread_id as u32))??;

        let frame_id = frame_info.frame_id;
        session
            .debugger_client
            .request_sync(move |debugger| debugger.set_frame_into_focus(frame_id as u32))??;

        type VarReqResult = anyhow::Result<Vec<(Identity, Value)>>;

        let variables = if var_ref.var_id == 0 {
            let variables = if var_ref.scope == variable::VarScope::Args {
                session
                    .debugger_client
                    .request_sync(|debugger| -> VarReqResult {
                        Ok(debugger
                            .read_argument(Dqe::Variable(Selector::Any))?
                            .into_iter()
                            .map(|v| v.into_identified_value())
                            .collect_vec())
                    })??
            } else {
                session
                    .debugger_client
                    .request_sync(|debugger| -> VarReqResult {
                        Ok(debugger
                            .read_variable(Dqe::Variable(Selector::Any))?
                            .into_iter()
                            .map(|v| v.into_identified_value())
                            .collect_vec())
                    })??
            };

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

            let variables = if var_ref.scope == variable::VarScope::Args {
                session
                    .debugger_client
                    .request_sync(|debugger| -> VarReqResult {
                        Ok(debugger
                            .read_argument(dqe)?
                            .into_iter()
                            .map(|v| v.into_identified_value())
                            .collect_vec())
                    })??
            } else {
                session
                    .debugger_client
                    .request_sync(|debugger| -> VarReqResult {
                        Ok(debugger
                            .read_variable(dqe)?
                            .into_iter()
                            .map(|v| v.into_identified_value())
                            .collect_vec())
                    })??
            };

            variables
                .into_iter()
                .flat_map(|(_, val)| {
                    variable::expand_and_collect(&mut self.var_ref_registry, var_ref, &path, &val)
                })
                .collect_vec()
        };

        self.server.respond_success(
            seq,
            ResponseBody::Variables(VariablesResponse { variables }),
        )?;

        Ok(())
    }
}

fn debugger_thread(
    program: String,
    cwd: Option<String>,
    debugger_builder: Arc<dyn Fn() -> DebuggerBuilder<DapHook>>,
    output: Arc<Mutex<ServerOutput<Stdout>>>,
    ready: &std::sync::Barrier,
    srv_exchanger: ServerExchanger,
) -> anyhow::Result<()> {
    let source = DebugeeSource::File {
        path: &program,
        args: &[],
        cwd: cwd.as_deref().map(Path::new),
    };

    let (stdout_reader, stdout_writer) = os_pipe::pipe()?;
    let (stderr_reader, stderr_writer) = os_pipe::pipe()?;

    let process = source.create_child(stdout_writer, stderr_writer)?;

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

    ready.wait();

    loop {
        match srv_exchanger.next_request() {
            Some(proto::Request::ExitSync) => {
                std::mem::drop(debugger);
                srv_exchanger.send_response(Box::new(()));
                break;
            }
            Some(proto::Request::DebuggerSyncTask(task)) => {
                let result = task(&mut debugger);
                srv_exchanger.send_response(result);
            }
            Some(proto::Request::DebuggerAsyncTask(task)) => {
                if let Err(e) = task(&mut debugger) {
                    srv_exchanger.send_async_response(e);
                }
            }
            None => {
                break;
            }
            Some(_) => {
                unreachable!("unexpected request");
            }
        }
    }

    log::debug!("Debugger thread exiting");

    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
struct FrameInfo {
    thread_id: u16,
    frame_id: u16,
}

impl FrameInfo {
    pub fn pack(self) -> u32 {
        ((self.thread_id as u32) << 16) | self.frame_id as u32
    }

    pub fn unpack(packed: i64) -> Self {
        let packed = packed as u64;
        Self {
            thread_id: (packed >> 16) as u16,
            frame_id: (packed & 0xFFFF) as u16,
        }
    }
}
