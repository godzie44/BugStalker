mod logger;

use std::io::{self, BufRead, BufReader, BufWriter, Stdin, Stdout};
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};

use anyhow::anyhow;
use dap::events::{Event, ExitedEventBody, OutputEventBody};
use dap::requests::{Command, Request};
use dap::responses::{ResponseBody, SetBreakpointsResponse, ThreadsResponse};
use dap::server::{Server, ServerOutput};
use dap::types::{Capabilities, OutputEventCategory};
use logger::DapLogger;
use nix::sys::signal::Signal::SIGKILL;

use crate::debugger::{DebuggerBuilder, EventHook};
use crate::ui::supervisor::DebugeeSource;

use super::supervisor;

pub struct DapApplication {
    debugger_builder: Arc<dyn Fn() -> DebuggerBuilder<DapHook> + Send + Sync>,
    server: Server<Stdin, Stdout>,
    session: Option<Session>,
}

struct Session {
    pid: nix::unistd::Pid,
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

            log::debug!("{}: {:?}", req.seq, req.command);

            match self.handle_request(req) {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => {
                    log::error!("{e}")
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
                        ..Default::default()
                    })))?;

                self.server.send_event(Event::Initialized)?;
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

                let (launched_sender, launched_receiver) = std::sync::mpsc::sync_channel(0);

                std::thread::spawn({
                    let debugger_builder: Arc<dyn Fn() -> DebuggerBuilder<DapHook> + Send + Sync> =
                        self.debugger_builder.clone();
                    let output = self.server.output.clone();

                    move || {
                        let result = debugger_thread(
                            program,
                            cwd,
                            debugger_builder,
                            output,
                            launched_sender,
                        );

                        if let Err(e) = result {
                            log::error!("{e}");
                        }
                    }
                });

                let pid = launched_receiver.recv().unwrap();

                self.session = Some(Session { pid });

                log::info!("launch successful");

                self.server.respond(req.success(ResponseBody::Launch))?;
            }
            Command::SetBreakpoints(_args) => {
                self.server
                    .respond(
                        req.success(ResponseBody::SetBreakpoints(SetBreakpointsResponse {
                            breakpoints: vec![],
                        })),
                    )?;
            }
            Command::Threads => {
                // TODO
                self.server.respond(
                    req.success(ResponseBody::Threads(ThreadsResponse { threads: vec![] })),
                )?;
            }
            Command::Disconnect(_) => {
                if let Some(session) = self.session.take() {
                    nix::sys::signal::kill(session.pid, SIGKILL)?;
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
        self.output
            .lock()
            .unwrap()
            .send_event(Event::Exited(ExitedEventBody {
                exit_code: code.into(),
            }))
            .unwrap();
    }

    fn on_process_install(&self, pid: thread_db::Pid, object: Option<&object::File>) {}
}

fn debugger_thread(
    program: String,
    cwd: Option<String>,
    debugger_builder: Arc<dyn Fn() -> DebuggerBuilder<DapHook>>,
    output: Arc<Mutex<ServerOutput<Stdout>>>,
    launched_sender: mpsc::SyncSender<nix::unistd::Pid>,
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

    std::thread::spawn({
        let output = output.clone();
        move || {
            let mut stream = BufReader::new(stdout_reader);
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
                        category: Some(OutputEventCategory::Stdout),
                        output: line,
                        ..Default::default()
                    }))
                    .unwrap();
            }
        }
    });

    std::thread::spawn({
        let output = output.clone();
        move || {
            let mut stream = BufReader::new(stderr_reader);
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
                        category: Some(OutputEventCategory::Stderr),
                        output: line,
                        ..Default::default()
                    }))
                    .unwrap();
            }
        }
    });

    launched_sender.send(pid)?;

    debugger.start_debugee()?;

    Ok(())
}
