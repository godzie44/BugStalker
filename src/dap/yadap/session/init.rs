use crate::dap::yadap::protocol::{DapRequest, InternalEvent};
use crate::dap::yadap::sourcemap::SourceMap;
use crate::debugger;
use crate::debugger::process::{Child, Installed};
use anyhow::{Context, anyhow};
use nix::unistd::Pid;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ModuleInfo {
    pub module: Value,
    pub source: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionMode {
    Launch,
    Attach,
}

impl super::DebugSession {
    pub(super) fn handle_initialize(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        self.initialized = true;
        let body = json!({
            "supportsConfigurationDoneRequest": true,
            "supportsAttachRequest": true,
            "supportsTerminateRequest": true,
            "supportsRestartRequest": true,
            "supportsRestartFrame": true,
            "supportsTerminateThreadsRequest": true,
            "supportsCancelRequest": true,
            "supportsSetVariable": true,
            "supportsSetExpression": true,
            "supportsStepBack": false,
            "supportsReverseContinue": false,
            "supportsStepInTargetsRequest": true,
            "supportsGotoTargetsRequest": true,
            "supportsCompletionsRequest": true,
            "supportsBreakpointLocationsRequest": true,
            "supportsEvaluateForHovers": true,
            "supportsPauseRequest": true,
            "supportsDisassembleRequest": true,
            "supportsSourceRequest": true,
            "supportsFunctionBreakpoints": true,
            "supportsInstructionBreakpoints": true,
            "supportsDataBreakpoints": true,
            "supportsConditionalBreakpoints": true,
            "supportsHitConditionalBreakpoints": true,
            "supportsLogPoints": true,
            "supportsReadMemoryRequest": true,
            "supportsWriteMemoryRequest": true,
            "supportsModulesRequest": true,
            "supportsLoadedSourcesRequest": true,
            "supportsRunInTerminalRequest": true,
            "supportsExceptionBreakpoints": true,
            "supportsExceptionInfoRequest": true,
            "exceptionBreakpointFilters": [
                {
                    "filter": super::EXCEPTION_FILTER_SIGNAL,
                    "label": "Signals",
                    "default": true,
                    "description": "Stop on debuggee signals.",
                },
                {
                    "filter": super::EXCEPTION_FILTER_PROCESS,
                    "label": "Process",
                    "default": true,
                    "description": "Stop when the debuggee process disappears.",
                },
            ],
        });
        self.send_success_body(req, body)?;
        self.send_event("initialized")
    }

    fn build_debugger_from_process(
        &mut self,
        process: Child<Installed>,
        stdout_reader: os_pipe::PipeReader,
        stderr_reader: os_pipe::PipeReader,
        oracles: &[String],
    ) -> anyhow::Result<()> {
        let oracles = self.resolve_oracles(oracles);

        let dbg = debugger::DebuggerBuilder::<debugger::NopHook>::new()
            .with_oracles(oracles)
            .build(process)
            .context("Build debugger")?;
        self.debugger = Some(dbg);

        self.start_output_forwarding(stdout_reader, stderr_reader);
        Ok(())
    }

    fn build_debugger(
        &mut self,
        program: &str,
        args: &[String],
        oracles: &[String],
    ) -> anyhow::Result<()> {
        let (stdout_reader, stdout_writer) = os_pipe::pipe().unwrap();
        let (stderr_reader, stderr_writer) = os_pipe::pipe().unwrap();

        let program_path = if !Path::new(program).exists() {
            which::which(program)?.to_string_lossy().to_string()
        } else {
            program.to_string()
        };

        let proc_tpl = Child::new(
            program_path,
            args,
            None::<PathBuf>,
            stdout_writer,
            stderr_writer,
        );
        let process = proc_tpl
            .install()
            .context("Initial process instantiation")?;

        self.build_debugger_from_process(process, stdout_reader, stderr_reader, oracles)
    }

    fn build_attached_debugger(&mut self, pid: Pid, oracles: &[String]) -> anyhow::Result<()> {
        let (stdout_reader, stdout_writer) = os_pipe::pipe().unwrap();
        let (stderr_reader, stderr_writer) = os_pipe::pipe().unwrap();
        let oracles = self.resolve_oracles(oracles);
        let dbg = debugger::DebuggerBuilder::<debugger::NopHook>::new()
            .with_oracles(oracles)
            .build_attached(pid, stdout_writer, stderr_writer)
            .context("Attach external process")?;

        self.debugger = Some(dbg);
        self.start_output_forwarding(stdout_reader, stderr_reader);
        Ok(())
    }

    fn emit_process_start(&mut self) -> anyhow::Result<()> {
        let dbg = self
            .debugger
            .as_ref()
            .ok_or_else(|| anyhow!("process event: debugger not initialized"))?;
        let process = dbg.process();
        let program = process.program().to_string();
        let name = Path::new(&program)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&program)
            .to_string();
        let pid = process.pid().as_raw() as i64;
        let start_method = match self.session_mode {
            Some(SessionMode::Attach) => "attach",
            _ => "launch",
        };
        let process_body = json!({
            "name": name,
            "systemProcessId": pid,
            "isLocalProcess": true,
            "startMethod": start_method,
        });

        self.enqueue_event(InternalEvent::Process { body: process_body });

        let module_id = pid.to_string();
        let module = json!({
            "id": module_id,
            "name": name,
            "path": program,
            "isOptimized": false,
            "isUserCode": true,
        });
        let source = json!({
            "name": name,
            "path": program,
        });
        self.module_info = Some(ModuleInfo {
            module: module.clone(),
            source: source.clone(),
        });
        self.enqueue_event(InternalEvent::Module {
            reason: "new",
            module,
        });
        self.enqueue_event(InternalEvent::LoadedSource {
            reason: "new",
            source,
        });
        Ok(())
    }

    pub(super) fn handle_launch(
        &mut self,
        req: &DapRequest,
        oracles: &[String],
    ) -> anyhow::Result<()> {
        let program = req
            .arguments
            .get("program")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("launch: missing arguments.program"))?;
        let args: Vec<String> = req
            .arguments
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        // Source map (remote/WSL/container path mapping).
        self.source_map = SourceMap::from_launch_args(&req.arguments);
        // Reset session termination state for a new launch.
        self.terminated = false;
        self.exit_code = None;
        self.session_mode = Some(SessionMode::Launch);

        let progress_id = self.enqueue_progress_start(
            "Launching debuggee",
            Some("Initializing debugger".to_string()),
            Some(0),
        );
        self.enqueue_capabilities(json!({ "supportsRestartRequest": true }));
        self.drain_events()?;
        self.build_debugger(program, &args, oracles)?;
        self.emit_process_start()?;
        self.enqueue_progress_update(
            progress_id.clone(),
            Some("Debugger initialized".to_string()),
            Some(50),
        );
        self.enqueue_progress_end(progress_id, Some("Launch complete".to_string()));
        self.send_success(req)?;
        self.drain_events()?;
        Ok(())
    }

    fn attach_pid(arguments: &Value) -> anyhow::Result<Pid> {
        let pid_value = arguments
            .get("pid")
            .or_else(|| arguments.get("processId"))
            .ok_or_else(|| anyhow!("attach: missing arguments.pid/processId"))?;

        let pid_raw = if let Some(pid) = pid_value.as_i64() {
            pid
        } else if let Some(pid_str) = pid_value.as_str() {
            pid_str
                .parse::<i64>()
                .map_err(|_| anyhow!("attach: pid must be an integer"))?
        } else {
            return Err(anyhow!("attach: pid must be an integer"));
        };

        let pid = i32::try_from(pid_raw).map_err(|_| anyhow!("attach: pid out of range"))?;
        Ok(Pid::from_raw(pid))
    }

    pub(super) fn handle_attach(
        &mut self,
        req: &DapRequest,
        oracles: &[String],
    ) -> anyhow::Result<()> {
        let pid = Self::attach_pid(&req.arguments)?;

        self.source_map = SourceMap::from_launch_args(&req.arguments);
        self.terminated = false;
        self.exit_code = None;
        self.session_mode = Some(SessionMode::Attach);

        let progress_id = self.enqueue_progress_start(
            "Attaching debuggee",
            Some("Initializing debugger".to_string()),
            Some(0),
        );
        self.enqueue_capabilities(json!({ "supportsRestartRequest": false }));
        self.drain_events()?;
        self.build_attached_debugger(pid, oracles)?;
        self.emit_process_start()?;
        self.enqueue_progress_update(
            progress_id.clone(),
            Some("Debugger attached".to_string()),
            Some(50),
        );
        self.enqueue_progress_end(progress_id, Some("Attach complete".to_string()));
        self.send_success(req)?;
        self.drain_events()?;
        Ok(())
    }

    fn emit_attached_stop(&mut self) -> anyhow::Result<()> {
        self.begin_stop_epoch();
        let _ = self.refresh_threads_with_events();

        let pid_info = self
            .debugger
            .as_ref()
            .map(|dbg| dbg.process().pid().as_raw());
        let description = pid_info.map(|pid| format!("Attached to process {pid}"));

        let (source_path, line, column, stack_trace) = self
            .debugger
            .as_ref()
            .and_then(|dbg| {
                let pid = dbg.ecx().pid_on_focus();
                let bt = dbg.backtrace(pid).unwrap_or_default();
                if bt.is_empty() {
                    return None;
                }
                let (source_path, line, column) = bt.first().and_then(|frame| {
                    frame.place.as_ref().map(|place| {
                        let path = place.file.to_string_lossy();
                        let mapped = self.source_map.map_target_to_client(path.as_ref());
                        (
                            Some(mapped),
                            Some(place.line_number as i64),
                            Some(place.column_number as i64),
                        )
                    })
                })?;
                let stack_trace = bt
                    .iter()
                    .enumerate()
                    .map(|(idx, frame)| {
                        let name = frame.func_name.as_deref().unwrap_or("<unknown>");
                        if let Some(place) = frame.place.as_ref() {
                            let path = place.file.to_string_lossy();
                            let mapped = self.source_map.map_target_to_client(path.as_ref());
                            format!(
                                "#{idx} {name} ({mapped}:{}:{})",
                                place.line_number, place.column_number
                            )
                        } else {
                            format!("#{idx} {name} (<unknown>)")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                Some((source_path, line, column, Some(stack_trace)))
            })
            .unwrap_or((None, None, None, None));

        self.last_stop = Some(super::control::LastStop {
            reason: "pause".to_string(),
            description: description.clone(),
            signal: None,
            source_path,
            line,
            column,
            stack_trace,
        });

        let thread_id = self.current_thread_id();
        self.enqueue_event(InternalEvent::Stopped {
            reason: "pause".to_string(),
            thread_id,
            description,
        });
        self.drain_events()?;
        Ok(())
    }

    pub(super) fn handle_configuration_done(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let dbg = self
            .debugger
            .as_mut()
            .ok_or_else(|| anyhow!("configurationDone: debugger not initialized"))?;

        if self.session_mode == Some(SessionMode::Attach) {
            self.send_success(req)?;
            self.emit_attached_stop()?;
            return Ok(());
        }

        let stop = dbg.start_debugee_with_reason().context("start debugee")?;
        self.send_success(req)?;
        self.emit_stop_reason(stop)?;
        Ok(())
    }
}
