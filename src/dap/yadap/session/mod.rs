//! DAP session implementation (handlers, state machine, and integration with debugger).

use crate::dap::transport::DapTransport;
use crate::dap::yadap::protocol::{self, DapRequest, DapResponse, InternalEvent};
use crate::dap::yadap::sourcemap::SourceMap;
use crate::debugger;
use crate::oracle::{Oracle, builtin};
use anyhow::{Context, anyhow};
use log::{info, warn};
use nix::unistd::Pid;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::sync::atomic::AtomicI64;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub mod breakpoint;
pub mod control;
pub mod data;
pub mod frame;
pub mod init;
pub mod other;
pub mod source;

pub struct DebugSession {
    io: Arc<Mutex<dyn DapTransport>>,
    server_seq: Arc<std::sync::atomic::AtomicI64>,
    initialized: bool,
    debugger: Option<debugger::Debugger>,
    session_mode: Option<init::SessionMode>,
    source_map: SourceMap,
    breakpoints_by_source: HashMap<String, Vec<breakpoint::BreakpointRecord>>,
    function_breakpoints: Vec<breakpoint::BreakpointRecord>,
    instruction_breakpoints: Vec<breakpoint::BreakpointRecord>,
    data_breakpoints: HashMap<String, breakpoint::DataBreakpointRecord>,
    thread_cache: HashMap<i64, Pid>,
    next_breakpoint_id: i64,
    vars: VariablesStore,
    scope_cache: HashMap<(i64, u32, frame::ScopeKind), i64>,
    child_links: HashMap<(i64, usize), i64>,
    disasm_cache_by_addr: HashMap<usize, source::DisasmSource>,
    disasm_cache_by_reference: HashMap<i64, source::DisasmSource>,
    next_source_reference: i64,
    events: Vec<InternalEvent>,
    next_progress_id: u64,
    terminated: bool,
    exit_code: Option<i32>,
    exception_filters: Vec<String>,
    last_stop: Option<control::LastStop>,
    module_info: Option<init::ModuleInfo>,
    canceled_request_ids: HashSet<i64>,
    canceled_progress_ids: HashSet<String>,
}

const EXCEPTION_FILTER_SIGNAL: &str = "signal";
const EXCEPTION_FILTER_PROCESS: &str = "process";
const DEBUGGER_RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
const MEMORY_READ_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
struct VariablesStore {
    next_ref: i64,
    store: HashMap<i64, Vec<data::VarItem>>,
}

impl VariablesStore {
    fn alloc(&mut self, vars: Vec<data::VarItem>) -> i64 {
        self.next_ref += 1;
        let key = self.next_ref;
        self.store.insert(key, vars);
        key
    }

    fn get(&self, key: i64) -> Option<&Vec<data::VarItem>> {
        self.store.get(&key)
    }

    fn get_mut(&mut self, key: i64) -> Option<&mut Vec<data::VarItem>> {
        self.store.get_mut(&key)
    }

    fn remove(&mut self, key: i64) -> Option<Vec<data::VarItem>> {
        self.store.remove(&key)
    }

    fn clear(&mut self) {
        self.next_ref = 1;
        self.store.clear();
    }
}

impl DebugSession {
    pub fn new(io: Arc<Mutex<dyn DapTransport>>) -> Self {
        Self {
            io,
            server_seq: Arc::new(AtomicI64::new(1)),
            initialized: false,
            debugger: None,
            session_mode: None,
            source_map: SourceMap::default(),
            breakpoints_by_source: HashMap::new(),
            function_breakpoints: Vec::new(),
            instruction_breakpoints: Vec::new(),
            data_breakpoints: HashMap::new(),
            thread_cache: HashMap::new(),
            next_breakpoint_id: 1,
            vars: VariablesStore::default(),
            scope_cache: HashMap::new(),
            child_links: HashMap::new(),
            disasm_cache_by_addr: HashMap::new(),
            disasm_cache_by_reference: HashMap::new(),
            next_source_reference: 1,
            events: Vec::new(),
            next_progress_id: 1,
            terminated: false,
            exit_code: None,
            exception_filters: vec![
                EXCEPTION_FILTER_SIGNAL.to_string(),
                EXCEPTION_FILTER_PROCESS.to_string(),
            ],
            last_stop: None,
            module_info: None,
            canceled_request_ids: HashSet::new(),
            canceled_progress_ids: HashSet::new(),
        }
    }

    fn next_seq(&mut self) -> i64 {
        self.server_seq
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    fn next_progress_id(&mut self) -> String {
        let id = self.next_progress_id;
        self.next_progress_id = self.next_progress_id.saturating_add(1);
        format!("bs-progress-{id}")
    }

    fn enqueue_event(&mut self, ev: InternalEvent) {
        self.events.push(ev);
    }

    fn enqueue_progress_start(
        &mut self,
        title: impl Into<String>,
        message: Option<String>,
        percentage: Option<u32>,
    ) -> String {
        let progress_id = self.next_progress_id();
        self.enqueue_event(InternalEvent::ProgressStart {
            progress_id: progress_id.clone(),
            title: title.into(),
            message,
            percentage,
        });
        progress_id
    }

    fn enqueue_progress_update(
        &mut self,
        progress_id: String,
        message: Option<String>,
        percentage: Option<u32>,
    ) {
        self.enqueue_event(InternalEvent::ProgressUpdate {
            progress_id,
            message,
            percentage,
        });
    }

    fn enqueue_progress_end(&mut self, progress_id: String, message: Option<String>) {
        self.enqueue_event(InternalEvent::ProgressEnd {
            progress_id,
            message,
        });
    }

    fn enqueue_invalidated(&mut self, areas: Vec<String>) {
        self.enqueue_event(InternalEvent::Invalidated { areas });
    }

    fn enqueue_capabilities(&mut self, capabilities: Value) {
        self.enqueue_event(InternalEvent::Capabilities { capabilities });
    }

    fn begin_stop_epoch(&mut self) {
        self.vars.clear();
        self.scope_cache.clear();
        self.child_links.clear();
    }

    fn begin_running(&mut self) {
        // Invalidate variable references when the program resumes.
        self.vars.clear();
        self.scope_cache.clear();
        self.child_links.clear();
    }

    fn enqueue_thread_event(&mut self, reason: &'static str, thread_id: i64) {
        self.enqueue_event(InternalEvent::Thread { reason, thread_id });
    }

    fn emit_process_end(&mut self) -> anyhow::Result<()> {
        if let Some(info) = self.module_info.take() {
            self.send_event_body(
                "module",
                json!({ "reason": "removed", "module": info.module }),
            )?;
            self.send_event_body(
                "loadedSource",
                json!({ "reason": "removed", "source": info.source }),
            )?;
        }
        let thread_ids: Vec<i64> = self.thread_cache.keys().copied().collect();
        for thread_id in thread_ids {
            self.send_event_body(
                "thread",
                json!({ "reason": "exited", "threadId": thread_id }),
            )?;
        }
        Ok(())
    }

    fn drain_events(&mut self) -> anyhow::Result<()> {
        // Drain queued internal events.
        let mut drained = Vec::new();
        drained.append(&mut self.events);

        // If we already terminated this session, ignore any late events (output/stopped etc.).
        if self.terminated {
            return Ok(());
        }

        // Lifecycle events must be deterministic and must dominate other event types.
        // If `Exited` or `Terminated` is present in the queue, we send the corresponding
        // DAP events once and ignore any other events from this drain batch.
        let mut exit_code: Option<i32> = None;
        let mut has_terminated = false;
        for ev in &drained {
            match ev {
                InternalEvent::Exited { code } => {
                    exit_code = Some(*code);
                }
                InternalEvent::Terminated => {
                    has_terminated = true;
                }
                _ => {}
            }
        }

        if let Some(code) = exit_code {
            self.send_events(|ev| matches!(ev, InternalEvent::Output { .. }), &drained)?;

            // Natural process exit: exited -> terminated (exactly once).
            self.emit_process_end()?;
            self.terminated = true;
            self.exit_code = Some(code);
            self.send_event_body("exited", json!({ "exitCode": code }))?;
            self.send_event("terminated")?;
            return Ok(());
        }

        if has_terminated {
            self.send_events(|ev| matches!(ev, InternalEvent::Output { .. }), &drained)?;

            // User-initiated termination: terminated only (exactly once).
            self.emit_process_end()?;
            self.terminated = true;
            self.send_event("terminated")?;
            return Ok(());
        }

        self.send_events(|_| true, &drained)
    }

    fn send_events<F: Fn(&InternalEvent) -> bool>(
        &mut self,
        filter: F,
        drained: &[InternalEvent],
    ) -> anyhow::Result<()> {
        for ev in drained {
            if !filter(ev) {
                continue;
            }

            match ev {
                InternalEvent::Stopped {
                    reason,
                    thread_id,
                    description,
                } => {
                    let body = json!({
                        "reason": reason,
                        "threadId": thread_id,
                        "allThreadsStopped": true,
                        "description": description,
                    });
                    self.send_event_raw("stopped", Some(body))?;
                }
                InternalEvent::Continued {
                    thread_id,
                    all_threads_continued,
                } => {
                    let body = if let Some(thread_id) = thread_id {
                        json!({
                            "threadId": thread_id,
                            "allThreadsContinued": all_threads_continued,
                        })
                    } else {
                        json!({
                            "allThreadsContinued": all_threads_continued,
                        })
                    };
                    self.send_event_raw("continued", Some(body))?;
                }
                InternalEvent::Thread { reason, thread_id } => {
                    self.send_event_body(
                        "thread",
                        json!({
                            "reason": reason,
                            "threadId": thread_id,
                        }),
                    )?;
                }
                InternalEvent::Breakpoint { reason, breakpoint } => {
                    self.send_event_body(
                        "breakpoint",
                        json!({
                            "reason": reason,
                            "breakpoint": breakpoint,
                        }),
                    )?;
                }
                InternalEvent::Module { reason, module } => {
                    self.send_event_body(
                        "module",
                        json!({
                            "reason": reason,
                            "module": module,
                        }),
                    )?;
                }
                InternalEvent::LoadedSource { reason, source } => {
                    self.send_event_body(
                        "loadedSource",
                        json!({
                            "reason": reason,
                            "source": source,
                        }),
                    )?;
                }
                InternalEvent::Process { body } => {
                    self.send_event_body("process", body)?;
                }
                InternalEvent::Exited { .. } | InternalEvent::Terminated => {
                    // Handled by the lifecycle pre-scan above.
                }
                InternalEvent::Output { category, output } => {
                    self.send_event_body(
                        "output",
                        json!({ "category": category, "output": output }),
                    )?;
                }
                InternalEvent::ProgressStart {
                    progress_id,
                    title,
                    message,
                    percentage,
                } => {
                    let mut body = json!({
                        "progressId": progress_id,
                        "title": title,
                    });
                    if let Some(message) = message {
                        body["message"] = json!(message);
                    }
                    if let Some(percentage) = percentage {
                        body["percentage"] = json!(percentage);
                    }
                    self.send_event_body("progressStart", body)?;
                }
                InternalEvent::ProgressUpdate {
                    progress_id,
                    message,
                    percentage,
                } => {
                    let mut body = json!({
                        "progressId": progress_id,
                    });
                    if let Some(message) = message {
                        body["message"] = json!(message);
                    }
                    if let Some(percentage) = percentage {
                        body["percentage"] = json!(percentage);
                    }
                    self.send_event_body("progressUpdate", body)?;
                }
                InternalEvent::ProgressEnd {
                    progress_id,
                    message,
                } => {
                    let mut body = json!({
                        "progressId": progress_id,
                    });
                    if let Some(message) = message {
                        body["message"] = json!(message);
                    }
                    self.send_event_body("progressEnd", body)?;
                }
                InternalEvent::Invalidated { areas } => {
                    let mut body = json!({});
                    if !areas.is_empty() {
                        body["areas"] = json!(areas);
                    }
                    self.send_event_body("invalidated", body)?;
                }
                InternalEvent::Capabilities { capabilities } => {
                    self.send_event_body("capabilities", json!({ "capabilities": capabilities }))?;
                }
            }
        }
        Ok(())
    }

    fn send_success(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        self.send_response_raw(req, true, None, None)
    }

    fn send_success_body<T: Serialize>(&mut self, req: &DapRequest, body: T) -> anyhow::Result<()> {
        let body = serde_json::to_value(body)?;
        self.send_response_raw(req, true, None, Some(body))
    }

    fn send_err(&mut self, req: &DapRequest, message: impl ToString) -> anyhow::Result<()> {
        self.send_response_raw(req, false, Some(message.to_string()), None)
    }

    fn send_cancelled(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        self.send_response_raw(
            req,
            false,
            Some("cancelled".to_string()),
            Some(json!({ "cancelled": true })),
        )
    }

    fn send_response_raw(
        &mut self,
        req: &DapRequest,
        success: bool,
        message: Option<String>,
        body: Option<Value>,
    ) -> anyhow::Result<()> {
        let rsp = DapResponse {
            seq: self.next_seq(),
            r#type: "response",
            request_seq: req.seq,
            success,
            command: req.command.clone(),
            message,
            body,
        };
        let value = serde_json::to_value(rsp)?;

        let mut lock = self.io.lock().unwrap();
        lock.write_message(&value)
    }

    fn send_event(&mut self, name: &'static str) -> anyhow::Result<()> {
        self.send_event_raw(name, None)
    }

    fn send_event_body<T: Serialize>(&mut self, name: &'static str, body: T) -> anyhow::Result<()> {
        let body = serde_json::to_value(body)?;
        self.send_event_raw(name, Some(body))
    }

    fn send_event_raw(&mut self, name: &'static str, body: Option<Value>) -> anyhow::Result<()> {
        let seq = self.next_seq();
        let mut lock = self.io.lock().unwrap();

        protocol::send_event(seq, &mut *lock, name, body)
    }

    fn consume_cancellation(
        &mut self,
        req: &DapRequest,
        progress_id: Option<&str>,
    ) -> anyhow::Result<bool> {
        let mut canceled = false;
        if self.canceled_request_ids.remove(&req.seq) {
            canceled = true;
        }
        if let Some(progress_id) = progress_id
            && self.canceled_progress_ids.remove(progress_id)
        {
            canceled = true;
        }

        if canceled {
            if let Some(progress_id) = progress_id {
                self.enqueue_progress_end(progress_id.to_string(), Some("Cancelled".to_string()));
                self.drain_events()?;
            }
            self.send_cancelled(req)?;
        }

        Ok(canceled)
    }

    fn resolve_oracles(&self, oracles: &[String]) -> Vec<Arc<dyn Oracle>> {
        oracles
            .iter()
            .filter_map(|ora_name| {
                if let Some(oracle) = builtin::make_builtin(ora_name) {
                    info!(target: "debugger", "oracle `{ora_name}` discovered");
                    Some(oracle)
                } else {
                    warn!(target: "debugger", "oracle `{ora_name}` not found");
                    None
                }
            })
            .collect()
    }

    fn start_output_forwarding(
        &self,
        stdout_reader: os_pipe::PipeReader,
        stderr_reader: os_pipe::PipeReader,
    ) {
        // Start stdout/stderr forwarding.
        let io = self.io.clone();
        let seq = self.server_seq.clone();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout_reader);
            let mut buf = String::new();
            loop {
                buf.clear();
                match reader.read_line(&mut buf) {
                    Ok(0) => break,
                    Ok(_) => {
                        let s = seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                        {
                            let mut lock = io.lock().unwrap();
                            // TODO log it somehow
                            _ = protocol::send_event(
                                s,
                                &mut *lock,
                                "output",
                                Some(json!({ "category": "stdout", "output": buf.clone() })),
                            );
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        let io = self.io.clone();
        let seq = self.server_seq.clone();

        thread::spawn(move || {
            let mut reader = BufReader::new(stderr_reader);
            let mut buf = String::new();
            loop {
                buf.clear();
                match reader.read_line(&mut buf) {
                    Ok(0) => break,
                    Ok(_) => {
                        let s = seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                        {
                            let mut lock = io.lock().unwrap();
                            // TODO log it somehow
                            _ = protocol::send_event(
                                s,
                                &mut *lock,
                                "output",
                                Some(json!({ "category": "stderr", "output": buf.clone() })),
                            );
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }

    fn decode_frame_id(frame_id: i64) -> (i64, u32) {
        let thread_id = frame_id >> 16;
        let frame = (frame_id & 0xFFFF) as u32;
        (thread_id, frame)
    }

    fn dispatch(&mut self, req: &DapRequest, oracles: &[String]) -> anyhow::Result<bool> {
        match req.command.as_str() {
            "initialize" => self.handle_initialize(req)?,
            "launch" => self.handle_launch(req, oracles)?,
            "attach" => self.handle_attach(req, oracles)?,
            "configurationDone" => self.handle_configuration_done(req)?,
            "setBreakpoints" => self.handle_set_breakpoints(req)?,
            "setFunctionBreakpoints" => self.handle_set_function_breakpoints(req)?,
            "setInstructionBreakpoints" => self.handle_set_instruction_breakpoints(req)?,
            "setExceptionBreakpoints" => self.handle_set_exception_breakpoints(req)?,
            "dataBreakpointInfo" => self.handle_data_breakpoint_info(req)?,
            "setDataBreakpoints" => self.handle_set_data_breakpoints(req)?,
            "breakpointLocations" => self.handle_breakpoint_locations(req)?,
            "exceptionInfo" => self.handle_exception_info(req)?,
            "threads" => self.handle_threads(req)?,
            "stackTrace" => self.handle_stack_trace(req)?,
            "scopes" => self.handle_scopes(req)?,
            "variables" => self.handle_variables(req)?,
            "setVariable" => self.handle_set_variable(req)?,
            "continue" => self.handle_continue(req)?,
            "restart" => self.handle_restart(req)?,
            "restartFrame" => self.handle_restart_frame(req)?,
            "next" => self.handle_next(req)?,
            "stepIn" => self.handle_step_in(req)?,
            "stepInTargets" => self.handle_step_in_targets(req)?,
            "stepOut" => self.handle_step_out(req)?,
            "stepBack" => self.handle_step_back(req)?,
            "reverseContinue" => self.handle_reverse_continue(req)?,
            "pause" => self.handle_pause(req)?,
            "gotoTargets" => self.handle_goto_targets(req)?,
            "goto" => self.handle_goto(req)?,
            "evaluate" => self.handle_evaluate(req)?,
            "setExpression" => self.handle_set_expression(req)?,
            "completions" => self.handle_completions(req)?,
            "loadedSources" => self.handle_loaded_sources(req)?,
            "modules" => self.handle_modules(req)?,
            "readMemory" => self.handle_read_memory(req)?,
            "writeMemory" => self.handle_write_memory(req)?,
            "disassemble" => self.handle_disassemble(req)?,
            "terminate" => {
                self.handle_terminate(req)?;
                return Ok(false);
            }
            "terminateThreads" => self.handle_terminate_threads(req)?,
            "cancel" => self.handle_cancel(req)?,
            "runInTerminal" => self.handle_run_in_terminal(req)?,
            "disconnect" => {
                self.handle_disconnect(req)?;
                return Ok(false);
            }
            "source" => {
                self.handle_source(req)?;
                return Ok(true);
            }
            other => {
                self.send_err(req, format!("Unsupported DAP command: {other}"))?;
            }
        }
        Ok(true)
    }

    pub fn run(mut self, oracles: Vec<String>) -> anyhow::Result<()> {
        loop {
            self.drain_events()?;

            let msg = {
                let mut lock = self.io.lock().unwrap();
                lock.read_message()?
            };
            let req: DapRequest = serde_json::from_value(msg)?;
            if req.r#type != "request" {
                continue;
            }
            let cont = match self.dispatch(&req, &oracles) {
                Ok(cont) => cont,
                Err(e) => {
                    let _ = self.send_err(&req, format!("{e:#}"));
                    true
                }
            };
            if !cont {
                break;
            }
        }
        Ok(())
    }
}

fn parse_memory_reference(reference: &str) -> anyhow::Result<usize> {
    let trimmed = reference.trim();
    let value = if let Some(hex) = trimmed.strip_prefix("0x") {
        usize::from_str_radix(hex, 16).context("parse hex memory reference")?
    } else {
        trimmed.parse::<usize>().context("parse memory reference")?
    };
    Ok(value)
}

fn parse_memory_reference_with_offset(reference: &str, offset: i64) -> anyhow::Result<usize> {
    let base = parse_memory_reference(reference)?;
    let base_i64 = i64::try_from(base).context("memoryReference out of range")?;
    let addr = base_i64
        .checked_add(offset)
        .ok_or_else(|| anyhow!("memoryReference + offset overflow"))?;
    if addr < 0 {
        anyhow::bail!("memoryReference + offset is negative");
    }
    usize::try_from(addr).context("memoryReference out of range")
}

/// Extension helper: set thread into focus by pid (DAP uses OS thread id as threadId).
trait ThreadFocusByPid {
    fn set_thread_into_focus_by_pid(&mut self, pid: Pid) -> Result<(), debugger::Error>;
}

impl ThreadFocusByPid for debugger::Debugger {
    fn set_thread_into_focus_by_pid(&mut self, pid: Pid) -> Result<(), debugger::Error> {
        // Find by pid in current thread list.
        let threads = self.thread_state()?;
        if let Some(t) = threads.into_iter().find(|t| t.thread.pid == pid) {
            let _ = self.set_thread_into_focus(t.thread.number);
        }
        Ok(())
    }
}
