//! BugStalker DAP (Debug Adapter Protocol) adapter.
//!
//! This binary exposes a minimal Debug Adapter Protocol server over TCP.
//! Intended as a building block for IDE integrations (VSCode, etc.).

use anyhow::{Context, anyhow};
use bugstalker::debugger;
use bugstalker::debugger::process::Child;
use bugstalker::oracle::builtin;
use clap::Parser;
use log::{info, warn};
use nix::unistd::Pid;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
struct Args {
	/// Address to listen on (default: 127.0.0.1:4711)
	#[clap(long, default_value = "127.0.0.1:4711")]
	listen: String,

	/// Discover a specific oracle (maybe more than one)
	#[clap(short, long)]
	oracle: Vec<String>,
}

/// DAP request envelope.
#[derive(Debug, Deserialize)]
struct DapRequest {
	seq: i64,
	#[serde(rename = "type")]
	r#type: String,
	command: String,
	#[serde(default)]
	arguments: Value,
}

/// DAP response envelope.
#[derive(Debug, Serialize)]
struct DapResponse<T: Serialize> {
	seq: i64,
	#[serde(rename = "type")]
	r#type: &'static str,
	request_seq: i64,
	success: bool,
	command: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	message: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	body: Option<T>,
}

/// DAP event envelope.
#[derive(Debug, Serialize)]
struct DapEvent<T: Serialize> {
	seq: i64,
	#[serde(rename = "type")]
	r#type: &'static str,
	event: &'static str,
	#[serde(skip_serializing_if = "Option::is_none")]
	body: Option<T>,
}

/// Small helper for DAP framing.
struct DapIo {
	stream: TcpStream,
	reader: BufReader<TcpStream>,
}

impl DapIo {
	fn new(stream: TcpStream) -> anyhow::Result<Self> {
		stream.set_nodelay(true)?;
		let reader = BufReader::new(stream.try_clone()?);
		Ok(Self { stream, reader })
	}

	fn read_message(&mut self) -> anyhow::Result<Value> {
		let mut content_length: Option<usize> = None;
		loop {
			let mut line = String::new();
			let read_n = self.reader.read_line(&mut line)?;
			if read_n == 0 {
				return Err(anyhow!("DAP connection closed"));
			}
			let line = line.trim_end_matches(|c| c == '\r' || c == '\n');
			if line.is_empty() {
				break;
			}
			if let Some(v) = line.strip_prefix("Content-Length:") {
				content_length = Some(v.trim().parse()?);
			}
		}

		let len = content_length.ok_or_else(|| anyhow!("Missing Content-Length header"))?;
		let mut buf = vec![0u8; len];
		self.reader.read_exact(&mut buf)?;
		let msg: Value = serde_json::from_slice(&buf)?;
		Ok(msg)
	}

	fn write_message<T: Serialize>(&mut self, v: &T) -> anyhow::Result<()> {
		let payload = serde_json::to_vec(v)?;
		write!(self.stream, "Content-Length: {}\r\n\r\n", payload.len())?;
		self.stream.write_all(&payload)?;
		self.stream.flush()?;
		Ok(())
	}
}

#[derive(Debug, Clone)]
enum InternalEvent {
	Stopped {
		reason: String,
		thread_id: Option<i64>,
		description: Option<String>,
	},
	Exited { code: i32 },
	Output { category: &'static str, output: String },
}

/// Debug session state for a single TCP client.
struct DebugSession {
	io: DapIo,
	server_seq: i64,
	initialized: bool,
	debugger: Option<debugger::Debugger>,
	breakpoints_by_source: HashMap<String, Vec<debugger::address::Address>>,
	thread_cache: HashMap<i64, Pid>,
	vars: VariablesStore,
	events: Arc<Mutex<Vec<InternalEvent>>>,
}

#[derive(Default)]
struct VariablesStore {
	next_ref: i64,
	store: HashMap<i64, Vec<VarItem>>,
}

#[derive(Clone)]
struct VarItem {
	name: String,
	value: String,
	type_name: Option<String>,
	child: Option<Vec<VarItem>>,
}

impl VariablesStore {
	fn alloc(&mut self, vars: Vec<VarItem>) -> i64 {
		self.next_ref += 1;
		let key = self.next_ref;
		self.store.insert(key, vars);
		key
	}
}

impl DebugSession {
	fn new(io: DapIo) -> Self {
		Self {
			io,
			server_seq: 1,
			initialized: false,
			debugger: None,
			breakpoints_by_source: HashMap::new(),
			thread_cache: HashMap::new(),
			vars: VariablesStore::default(),
			events: Arc::new(Mutex::new(Vec::new())),
		}
	}

	fn next_seq(&mut self) -> i64 {
		let s = self.server_seq;
		self.server_seq += 1;
		s
	}

	fn enqueue_event(&self, ev: InternalEvent) {
		if let Ok(mut q) = self.events.lock() {
			q.push(ev);
		}
	}

	fn drain_events(&mut self) -> anyhow::Result<()> {
		let mut drained = Vec::new();
		if let Ok(mut q) = self.events.lock() {
			drained.append(&mut *q);
		}
		for ev in drained {
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
					let ev = DapEvent::<Value> {
						seq: self.next_seq(),
						r#type: "event",
						event: "stopped",
						body: Some(body),
					};
					self.io.write_message(&ev)?;
				}
				InternalEvent::Exited { code } => {
					let ev = DapEvent::<Value> {
						seq: self.next_seq(),
						r#type: "event",
						event: "exited",
						body: Some(json!({"exitCode": code})),
					};
					self.io.write_message(&ev)?;
					let ev = DapEvent::<Value> {
						seq: self.next_seq(),
						r#type: "event",
						event: "terminated",
						body: Some(json!({"restart": false})),
					};
					self.io.write_message(&ev)?;
				}
				InternalEvent::Output { category, output } => {
					let ev = DapEvent::<Value> {
						seq: self.next_seq(),
						r#type: "event",
						event: "output",
						body: Some(json!({"category": category, "output": output})),
					};
					self.io.write_message(&ev)?;
				}
			}
		}
		Ok(())
	}

	fn send_ok<T: Serialize>(&mut self, req: &DapRequest, body: Option<T>) -> anyhow::Result<()> {
		let rsp = DapResponse {
			seq: self.next_seq(),
			r#type: "response",
			request_seq: req.seq,
			success: true,
			command: req.command.clone(),
			message: None,
			body,
		};
		self.io.write_message(&rsp)
	}

	fn send_err(&mut self, req: &DapRequest, message: impl ToString) -> anyhow::Result<()> {
		let rsp = DapResponse::<Value> {
			seq: self.next_seq(),
			r#type: "response",
			request_seq: req.seq,
			success: false,
			command: req.command.clone(),
			message: Some(message.to_string()),
			body: None,
		};
		self.io.write_message(&rsp)
	}

	fn handle_initialize(&mut self, req: &DapRequest) -> anyhow::Result<()> {
		self.initialized = true;
		let body = json!({
			"supportsConfigurationDoneRequest": true,
			"supportsTerminateRequest": true,
			"supportsRestartRequest": false,
			"supportsSetVariable": false,
			"supportsStepBack": false,
		});
		self.send_ok(req, Some(body))?;
		let ev = DapEvent::<Value> {
			seq: self.next_seq(),
			r#type: "event",
			event: "initialized",
			body: None,
		};
		self.io.write_message(&ev)
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

		let proc_tpl = Child::new(program_path, args, stdout_writer, stderr_writer);
		let process = proc_tpl.install().context("Initial process instantiation")?;

		let oracles = oracles
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
			.collect();

		let dbg = debugger::DebuggerBuilder::<debugger::NopHook>::new()
			.with_oracles(oracles)
			.build(process)
			.context("Build debugger")?;
		self.debugger = Some(dbg);

		// Start stdout/stderr forwarding.
		let evq = self.events.clone();
		thread::spawn(move || {
			let mut reader = BufReader::new(stdout_reader);
			let mut buf = String::new();
			loop {
				buf.clear();
				match reader.read_line(&mut buf) {
					Ok(0) => break,
					Ok(_) => {
						if let Ok(mut q) = evq.lock() {
							q.push(InternalEvent::Output {
								category: "stdout",
								output: buf.clone(),
							});
						}
					}
					Err(_) => break,
				}
			}
		});

		let evq = self.events.clone();
		thread::spawn(move || {
			let mut reader = BufReader::new(stderr_reader);
			let mut buf = String::new();
			loop {
				buf.clear();
				match reader.read_line(&mut buf) {
					Ok(0) => break,
					Ok(_) => {
						if let Ok(mut q) = evq.lock() {
							q.push(InternalEvent::Output {
								category: "stderr",
								output: buf.clone(),
							});
						}
					}
					Err(_) => break,
				}
			}
		});

		Ok(())
	}

	fn handle_launch(&mut self, req: &DapRequest, oracles: &[String]) -> anyhow::Result<()> {
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

		self.build_debugger(program, &args, oracles)?;
		self.send_ok(req, Some(json!({})))?;
		Ok(())
	}

	fn handle_configuration_done(&mut self, req: &DapRequest) -> anyhow::Result<()> {
		let dbg = self
			.debugger
			.as_mut()
			.ok_or_else(|| anyhow!("configurationDone: debugger not initialized"))?;

		let stop = dbg.start_debugee_with_reason().context("start debugee")?;
		self.send_ok(req, Some(json!({})))?;
		self.emit_stop_reason(stop)?;
		Ok(())
	}

	fn emit_stop_reason(&mut self, stop: debugger::StopReason) -> anyhow::Result<()> {
		let (reason, thread_id, description, exited) = match stop {
			debugger::StopReason::DebugeeExit(code) => {
				("exited".to_string(), None, None, Some(code))
			}
			debugger::StopReason::DebugeeStart => (
				"entry".to_string(),
				self.current_thread_id(),
				Some("Debugee started".to_string()),
				None,
			),
			debugger::StopReason::Breakpoint(pid, _) => (
				"breakpoint".to_string(),
				Some(pid.as_raw() as i64),
				None,
				None,
			),
			debugger::StopReason::Watchpoint(pid, _, _) => (
				"data breakpoint".to_string(),
				Some(pid.as_raw() as i64),
				None,
				None,
			),
			debugger::StopReason::SignalStop(pid, sign) => (
				"signal".to_string(),
				Some(pid.as_raw() as i64),
				Some(format!("Signal: {sign:?}")),
				None,
			),
			debugger::StopReason::NoSuchProcess(pid) => (
				"exception".to_string(),
				Some(pid.as_raw() as i64),
				Some("No such process".to_string()),
				None,
			),
		};

		if let Some(code) = exited {
			self.enqueue_event(InternalEvent::Exited { code });
			self.drain_events()?;
			return Ok(());
		}

		self.enqueue_event(InternalEvent::Stopped {
			reason,
			thread_id,
			description,
		});
		self.drain_events()?;
		Ok(())
	}

	fn current_thread_id(&mut self) -> Option<i64> {
		self.debugger
			.as_ref()
			.map(|d| d.exploration_ctx().pid_on_focus().as_raw() as i64)
	}

	fn refresh_threads(&mut self) -> anyhow::Result<Vec<Value>> {
		let dbg = self
			.debugger
			.as_ref()
			.ok_or_else(|| anyhow!("threads: debugger not initialized"))?;

		let threads = dbg.thread_state().unwrap_or_default();
		self.thread_cache.clear();
		let mut out = Vec::new();
		for t in threads {
			let id = t.thread.pid.as_raw() as i64;
			self.thread_cache.insert(id, t.thread.pid);
			out.push(json!({
				"id": id,
				"name": format!("thread#{} ({})", t.thread.number, t.thread.pid),
			}));
		}
		Ok(out)
	}

	fn handle_threads(&mut self, req: &DapRequest) -> anyhow::Result<()> {
		let threads = self.refresh_threads()?;
		self.send_ok(req, Some(json!({"threads": threads})))
	}

	fn handle_set_breakpoints(&mut self, req: &DapRequest) -> anyhow::Result<()> {
		let dbg = self
			.debugger
			.as_mut()
			.ok_or_else(|| anyhow!("setBreakpoints: debugger not initialized"))?;

		let source_path = req
			.arguments
			.get("source")
			.and_then(|s| s.get("path"))
			.and_then(|p| p.as_str())
			.ok_or_else(|| anyhow!("setBreakpoints: missing arguments.source.path"))?
			.to_string();

		// Remove previous breakpoints for this source.
		if let Some(prev) = self.breakpoints_by_source.remove(&source_path) {
			for addr in prev {
				let _ = dbg.remove_breakpoint(addr);
			}
		}

		let mut new_addrs = Vec::new();
		let mut rsp_bps = Vec::new();
		let bps = req
			.arguments
			.get("breakpoints")
			.and_then(|v| v.as_array())
			.cloned()
			.unwrap_or_default();

		for bp in bps {
			let line = bp.get("line").and_then(|v| v.as_i64()).unwrap_or(1) as u64;
			let mut views = dbg.set_breakpoint_at_line(&source_path, line);
			if views.is_err() {
				// fallback: try basename, helps when debug info stores only file name
				if let Some(base) = Path::new(&source_path).file_name().and_then(|s| s.to_str()) {
					views = dbg.set_breakpoint_at_line(base, line);
				}
			}

			match views {
				Ok(mut v) if !v.is_empty() => {
					let first = v.remove(0);
					new_addrs.push(first.addr);
					rsp_bps.push(json!({
						"verified": true,
						"line": line,
					}));
				}
				_ => {
					rsp_bps.push(json!({
						"verified": false,
						"line": line,
					}));
				}
			}
		}

		self.breakpoints_by_source.insert(source_path, new_addrs);
		self.send_ok(req, Some(json!({"breakpoints": rsp_bps})))
	}

	fn handle_continue(&mut self, req: &DapRequest) -> anyhow::Result<()> {
		let dbg = self
			.debugger
			.as_mut()
			.ok_or_else(|| anyhow!("continue: debugger not initialized"))?;
		let stop = dbg.continue_debugee_with_reason().context("continue")?;
		self.send_ok(req, Some(json!({"allThreadsContinued": true})))?;
		self.emit_stop_reason(stop)
	}

	fn handle_stack_trace(&mut self, req: &DapRequest) -> anyhow::Result<()> {
		let dbg = self
			.debugger
			.as_ref()
			.ok_or_else(|| anyhow!("stackTrace: debugger not initialized"))?;
		let thread_id = req
			.arguments
			.get("threadId")
			.and_then(|v| v.as_i64())
			.ok_or_else(|| anyhow!("stackTrace: missing arguments.threadId"))?;

		let pid = self
			.thread_cache
			.get(&thread_id)
			.copied()
			.unwrap_or_else(|| Pid::from_raw(thread_id as i32));

		let bt = dbg.backtrace(pid).unwrap_or_default();
		let mut frames = Vec::new();
		for (i, f) in bt.iter().enumerate() {
			let (path, line, col) = match f.place.as_ref() {
				Some(p) => (
					Some(p.file.to_string_lossy().to_string()),
					Some(p.line_number as i64),
					Some(p.column_number as i64),
				),
				None => (None, None, None),
			};
			let name = f
				.func_name
				.as_deref()
				.unwrap_or("<unknown>")
				.to_string();
			let frame_id = ((thread_id as i64) << 16) | (i as i64);
			let source = path.map(|p| json!({"path": p}));
			frames.push(json!({
				"id": frame_id,
				"name": name,
				"source": source,
				"line": line.unwrap_or(0),
				"column": col.unwrap_or(0),
			}));
		}
		self.send_ok(req, Some(json!({"stackFrames": frames, "totalFrames": frames.len()})))
	}

	fn decode_frame_id(frame_id: i64) -> (i64, u32) {
		let thread_id = frame_id >> 16;
		let frame = (frame_id & 0xFFFF) as u32;
		(thread_id, frame)
	}

	fn handle_scopes(&mut self, req: &DapRequest) -> anyhow::Result<()> {
		let dbg = self
			.debugger
			.as_mut()
			.ok_or_else(|| anyhow!("scopes: debugger not initialized"))?;

		let frame_id = req
			.arguments
			.get("frameId")
			.and_then(|v| v.as_i64())
			.ok_or_else(|| anyhow!("scopes: missing arguments.frameId"))?;
		let (thread_id, frame_num) = Self::decode_frame_id(frame_id);
		let pid = self
			.thread_cache
			.get(&thread_id)
			.copied()
			.unwrap_or_else(|| Pid::from_raw(thread_id as i32));

		// Focus selected thread/frame to make variable evaluation consistent.
		let _ = dbg.set_thread_into_focus_by_pid(pid);
		let _ = dbg.set_frame_into_focus(frame_num);

		// NOTE: avoid borrowing `self` mutably while `dbg` is borrowed.
		let locals = read_locals(dbg).unwrap_or_default();
		let args = read_args(dbg).unwrap_or_default();
		let locals_ref = self.vars.alloc(locals);
		let args_ref = self.vars.alloc(args);

		let scopes = vec![
			json!({"name": "Locals", "variablesReference": locals_ref, "expensive": false}),
			json!({"name": "Arguments", "variablesReference": args_ref, "expensive": false}),
		];

		self.send_ok(req, Some(json!({"scopes": scopes})))
	}

	fn handle_variables(&mut self, req: &DapRequest) -> anyhow::Result<()> {
		let variables_reference = req
			.arguments
			.get("variablesReference")
			.and_then(|v| v.as_i64())
			.ok_or_else(|| anyhow!("variables: missing arguments.variablesReference"))?;

		let vars = self
			.vars
			.store
			.get(&variables_reference)
			.cloned()
			.unwrap_or_default();

		let mut out = Vec::new();
		for v in vars {
			let child_ref = v
				.child
				.as_ref()
				.map(|c| self.vars.alloc(c.clone()))
				.unwrap_or(0);
			out.push(json!({
				"name": v.name,
				"value": v.value,
				"type": v.type_name,
				"variablesReference": child_ref,
			}));
		}

		self.send_ok(req, Some(json!({"variables": out})))
	}

	fn handle_disconnect(&mut self, req: &DapRequest) -> anyhow::Result<()> {
		self.send_ok(req, Some(json!({})))?;
		Ok(())
	}

	fn dispatch(&mut self, req: &DapRequest, oracles: &[String]) -> anyhow::Result<bool> {
		match req.command.as_str() {
			"initialize" => self.handle_initialize(req)?,
			"launch" => self.handle_launch(req, oracles)?,
			"configurationDone" => self.handle_configuration_done(req)?,
			"setBreakpoints" => self.handle_set_breakpoints(req)?,
			"threads" => self.handle_threads(req)?,
			"stackTrace" => self.handle_stack_trace(req)?,
			"scopes" => self.handle_scopes(req)?,
			"variables" => self.handle_variables(req)?,
			"continue" => self.handle_continue(req)?,
			"disconnect" => {
				self.handle_disconnect(req)?;
				return Ok(false);
			}
			other => {
				self.send_err(req, format!("Unsupported DAP command: {other}"))?;
			}
		}
		Ok(true)
	}

	fn run(mut self, oracles: Vec<String>) -> anyhow::Result<()> {
		loop {
			self.drain_events()?;
			let msg = self.io.read_message()?;
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

fn render_value_to_string(v: &debugger::variable::value::Value) -> String {
	use debugger::variable::render::RenderValue;
	match v.value_layout() {
		Some(debugger::variable::render::ValueLayout::PreRendered(s)) => s.to_string(),
		Some(debugger::variable::render::ValueLayout::Referential(ptr)) => {
			format!("{ptr:p}")
		}
		Some(debugger::variable::render::ValueLayout::Wrapped(inner)) => render_value_to_string(inner),
		Some(debugger::variable::render::ValueLayout::Structure(_)) => "{...}".to_string(),
		Some(debugger::variable::render::ValueLayout::IndexedList(_)) => "[...]".to_string(),
		Some(debugger::variable::render::ValueLayout::NonIndexedList(_)) => "[...]".to_string(),
		Some(debugger::variable::render::ValueLayout::Map(_)) => "{...}".to_string(),
		None => "<unavailable>".to_string(),
	}
}

fn value_children(v: &debugger::variable::value::Value) -> Option<Vec<VarItem>> {
	use debugger::variable::render::{RenderValue, ValueLayout};
	let layout = v.value_layout()?;
	match layout {
		ValueLayout::Structure(members) => {
			let mut out = Vec::new();
			for m in members {
				let field_name = m
					.field_name
					.as_deref()
					.unwrap_or("<unnamed>")
					.to_string();
				let val = &m.value;
				out.push(VarItem {
					name: field_name,
					value: render_value_to_string(val),
					type_name: Some(val.r#type().to_string()),
					child: value_children(val),
				});
			}
			Some(out)
		}
		ValueLayout::IndexedList(items) => {
			let mut out = Vec::new();
			for it in items {
				let val = &it.value;
				out.push(VarItem {
					name: format!("[{}]", it.index),
					value: render_value_to_string(val),
					type_name: Some(val.r#type().to_string()),
					child: value_children(val),
				});
			}
			Some(out)
		}
		ValueLayout::NonIndexedList(items) => {
			let mut out = Vec::new();
			for (i, val) in items.iter().enumerate() {
				out.push(VarItem {
					name: format!("[{i}]"),
					value: render_value_to_string(val),
					type_name: Some(val.r#type().to_string()),
					child: value_children(val),
				});
			}
			Some(out)
		}
		ValueLayout::Map(kvs) => {
			let mut out = Vec::new();
			for (i, (k, val)) in kvs.iter().enumerate() {
				out.push(VarItem {
					name: format!("[{i}]"),
					value: format!("{} => {}", render_value_to_string(k), render_value_to_string(val)),
					type_name: None,
					child: None,
				});
			}
			Some(out)
		}
		_ => None,
	}
}

fn read_locals(dbg: &debugger::Debugger) -> anyhow::Result<Vec<VarItem>> {
	use debugger::variable::render::RenderValue;
	let locals = dbg.read_local_variables()?;
	let mut out = Vec::new();
	for r in locals {
		let (id, val) = r.into_identified_value();
		let name = id.to_string();
		out.push(VarItem {
			name,
			value: render_value_to_string(&val),
			type_name: Some(val.r#type().to_string()),
			child: value_children(&val),
		});
	}
	Ok(out)
}

fn read_args(dbg: &debugger::Debugger) -> anyhow::Result<Vec<VarItem>> {
	use debugger::variable::dqe::{Dqe, Selector};
	use debugger::variable::render::RenderValue;
	let args = dbg.read_argument(Dqe::Variable(Selector::Any))?;
	let mut out = Vec::new();
	for r in args {
		let (id, val) = r.into_identified_value();
		let name = id.to_string();
		out.push(VarItem {
			name,
			value: render_value_to_string(&val),
			type_name: Some(val.r#type().to_string()),
			child: value_children(&val),
		});
	}
	Ok(out)
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

fn main() -> anyhow::Result<()> {
	let logger = env_logger::Logger::from_default_env();
	let filter = logger.filter();
	bugstalker::log::LOGGER_SWITCHER.switch(logger, filter);

	bugstalker::debugger::rust::Environment::init(None);
	let args = Args::parse();
	let addr: SocketAddr = args.listen.parse().context("Invalid listen address")?;
	let listener = TcpListener::bind(addr).with_context(|| format!("bind {addr}"))?;
	info!(target: "dap", "bs-dap listening on {addr}");

	// Single-client MVP: accept one connection, run session, then exit.
	let (stream, peer) = listener.accept().context("accept")?;
	info!(target: "dap", "DAP client connected: {peer}");
	let io = DapIo::new(stream)?;
	DebugSession::new(io).run(args.oracle)
}
