//! BugStalker DAP (Debug Adapter Protocol) adapter.
//!
//! This binary exposes a minimal Debug Adapter Protocol server over TCP.
//! Intended as a building block for IDE integrations (VSCode, etc.).

use anyhow::{Context, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_ENGINE};
use bugstalker::debugger;
use bugstalker::debugger::address::{GlobalAddress, RelocatedAddress};
use bugstalker::debugger::process::{Child, Installed};
use bugstalker::debugger::register::debug::{BreakCondition, BreakSize};
use bugstalker::debugger::variable::dqe::{Dqe, Literal};
use bugstalker::debugger::variable::render::RenderValue;
use bugstalker::oracle::{Oracle, builtin};
use bugstalker::ui::command::parser::expression as bs_expr;
use bugstalker::ui::command::parser::watchpoint_at_address;
use bugstalker::ui::command::watch::WatchpointIdentity;
use capstone::prelude::*;
use chumsky::Parser as _;
use chumsky::prelude::end;
use clap::Parser;
use log::{info, warn};
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use regex::escape as regex_escape;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Address to listen on (default: 127.0.0.1:4711)
    #[clap(long, default_value = "127.0.0.1:4711")]
    listen: String,

    /// Exit after the first debug session ends (single-client mode).
    #[clap(long)]
    oneshot: bool,

    /// Optional log file for adapter diagnostics (no output to stdout).
    #[clap(long)]
    log_file: Option<std::path::PathBuf>,

    /// Trace DAP traffic (requests/responses/events) into the log file.
    /// Requires --log-file.
    #[clap(long)]
    trace_dap: bool,

    /// Discover a specific oracle (maybe more than one)
    #[clap(short, long)]
    oracle: Vec<String>,
}

/// Simple file-based tracer for adapter diagnostics.
#[derive(Clone)]
struct FileTracer {
    file: Arc<Mutex<std::fs::File>>,
}

impl FileTracer {
    fn new(path: &std::path::Path) -> anyhow::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("open log file {}", path.display()))?;
        Ok(Self {
            file: Arc::new(Mutex::new(file)),
        })
    }

    fn line(&self, text: &str) {
        if let Ok(mut file) = self.file.lock() {
            let _ = writeln!(file, "{text}");
        }
    }
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
///
/// Note: the DAP specification allows responses with no `body` field at all.
/// Using a `serde_json::Value` keeps the envelope stable and avoids type
/// inference issues around `None` bodies.
#[derive(Debug, Serialize)]
struct DapResponse {
    seq: i64,
    #[serde(rename = "type")]
    r#type: &'static str,
    request_seq: i64,
    success: bool,
    command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<Value>,
}

/// DAP event envelope.
#[derive(Debug, Serialize)]
struct DapEvent {
    seq: i64,
    #[serde(rename = "type")]
    r#type: &'static str,
    event: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<Value>,
}

/// Small helper for DAP framing.
struct DapIo {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
    tracer: Option<FileTracer>,
    trace: bool,
}

impl DapIo {
    fn new(stream: TcpStream, tracer: Option<FileTracer>, trace: bool) -> anyhow::Result<Self> {
        stream.set_nodelay(true)?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self {
            stream,
            reader,
            tracer,
            trace,
        })
    }

    fn read_message(&mut self) -> anyhow::Result<Value> {
        let mut content_length: Option<usize> = None;
        loop {
            let mut line = String::new();
            let read_n = self.reader.read_line(&mut line)?;
            if read_n == 0 {
                return Err(anyhow!("DAP connection closed"));
            }
            let line = line.trim_end_matches(['\r', '\n']);
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
        if self.trace
            && let Some(tracer) = &self.tracer
            && let Ok(line) = serde_json::to_string(&msg)
        {
            tracer.line(&format!("<- {line}"));
        }
        Ok(msg)
    }

    fn write_message<T: Serialize>(&mut self, v: &T) -> anyhow::Result<()> {
        let payload = serde_json::to_vec(v)?;
        if self.trace
            && let Some(tracer) = &self.tracer
            && let Ok(line) = serde_json::to_string(v)
        {
            tracer.line(&format!("-> {line}"));
        }
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
    Continued {
        thread_id: Option<i64>,
        all_threads_continued: bool,
    },
    Thread {
        reason: &'static str,
        thread_id: i64,
    },
    Breakpoint {
        reason: &'static str,
        breakpoint: Value,
    },
    Module {
        reason: &'static str,
        module: Value,
    },
    LoadedSource {
        reason: &'static str,
        source: Value,
    },
    Process {
        body: Value,
    },
    Exited {
        code: i32,
    },
    Terminated,
    Output {
        category: &'static str,
        output: String,
    },
}

#[derive(Debug, Default, Clone)]
struct SourceMap {
    /// Mapping from debuggee/DWARF paths to the client (VSCode) paths.
    target_to_client: Vec<(String, String)>,
    /// Reverse mapping from client (VSCode) paths to debuggee/DWARF paths.
    client_to_target: Vec<(String, String)>,
}

impl SourceMap {
    fn from_launch_args(arguments: &Value) -> Self {
        let mut sm = SourceMap::default();
        let Some(serde_json::Value::Object(map)) = arguments.get("sourceMap") else {
            return sm;
        };

        // Convention: key = target prefix, value = client prefix.
        for (target_prefix, client_prefix_val) in map.iter() {
            let Some(client_prefix) = client_prefix_val.as_str() else {
                continue;
            };

            let target_norm = Self::norm_prefix(target_prefix);
            let client_norm = Self::norm_prefix(client_prefix);

            sm.target_to_client
                .push((target_norm.clone(), client_prefix.to_string()));
            sm.client_to_target
                .push((client_norm, target_prefix.to_string()));
        }

        // Longest prefix wins.
        sm.target_to_client
            .sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        sm.client_to_target
            .sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        sm
    }

    fn map_target_to_client(&self, target_path: &str) -> String {
        self.apply_map(target_path, &self.target_to_client)
    }

    fn map_client_to_target(&self, client_path: &str) -> String {
        self.apply_map(client_path, &self.client_to_target)
    }

    fn apply_map(&self, path: &str, mapping: &[(String, String)]) -> String {
        let normalized = Self::norm_path(path);
        for (from_norm, to_raw) in mapping {
            if normalized.starts_with(from_norm) {
                let suffix = &normalized[from_norm.len()..];
                return Self::join_with_style(to_raw, suffix);
            }
        }
        path.to_string()
    }

    fn join_with_style(prefix: &str, suffix_norm: &str) -> String {
        if suffix_norm.is_empty() {
            return prefix.to_string();
        }
        let mut out = prefix.to_string();

        // Avoid double separators.
        let need_sep = !out.ends_with('/') && !out.ends_with('\\');
        if need_sep {
            // Pick separator style by prefix.
            out.push(if out.contains('\\') { '\\' } else { '/' });
        }

        let mut suffix = suffix_norm.to_string();
        // Convert suffix separators to match prefix style.
        if out.contains('\\') {
            suffix = suffix.replace('/', "\\");
        }
        out.push_str(&suffix);
        out
    }

    fn norm_prefix(s: &str) -> String {
        let mut out = Self::norm_path(s);
        if !out.ends_with('/') {
            out.push('/');
        }
        out
    }

    fn norm_path(s: &str) -> String {
        s.replace('\\', "/")
    }
}

/// Debug session state for a single TCP client.
struct DebugSession {
    io: DapIo,
    server_seq: i64,
    initialized: bool,
    debugger: Option<debugger::Debugger>,
    session_mode: Option<SessionMode>,
    source_map: SourceMap,
    breakpoints_by_source: HashMap<String, Vec<BreakpointRecord>>,
    function_breakpoints: Vec<BreakpointRecord>,
    instruction_breakpoints: Vec<BreakpointRecord>,
    data_breakpoints: HashMap<String, DataBreakpointRecord>,
    thread_cache: HashMap<i64, Pid>,
    next_breakpoint_id: i64,
    vars: VariablesStore,
    scope_cache: HashMap<(i64, u32, ScopeKind), i64>,
    child_links: HashMap<(i64, usize), i64>,
    disasm_cache_by_addr: HashMap<usize, DisasmSource>,
    disasm_cache_by_reference: HashMap<i64, DisasmSource>,
    next_source_reference: i64,
    events: Arc<Mutex<Vec<InternalEvent>>>,
    terminated: bool,
    exit_code: Option<i32>,
    exception_filters: Vec<String>,
    last_stop: Option<LastStop>,
    module_info: Option<ModuleInfo>,
}

const EXCEPTION_FILTER_SIGNAL: &str = "signal";
const EXCEPTION_FILTER_PROCESS: &str = "process";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ScopeKind {
    Locals,
    Arguments,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionMode {
    Launch,
    Attach,
}

#[derive(Debug, Clone)]
struct LastStop {
    reason: String,
    description: Option<String>,
    signal: Option<i32>,
    source_path: Option<String>,
    line: Option<i64>,
    column: Option<i64>,
    stack_trace: Option<String>,
}

#[derive(Debug, Clone)]
struct DisasmSource {
    reference: i64,
    name: String,
    content: String,
}

#[derive(Debug, Clone)]
struct BreakpointRecord {
    id: i64,
    addresses: Vec<debugger::address::Address>,
    condition: Option<String>,
    hit_condition: Option<HitCondition>,
    log_message: Option<String>,
    hit_count: u64,
}

#[derive(Debug, Clone)]
enum HitCondition {
    Exact(u64),
    GreaterOrEqual(u64),
    Greater(u64),
    Less(u64),
    LessOrEqual(u64),
    Invalid(String),
}

impl HitCondition {
    fn parse(input: &str) -> Self {
        let trimmed = input.trim();
        let parse_num = |s: &str| s.trim().parse::<u64>();
        if let Some(rest) = trimmed.strip_prefix(">=") {
            return parse_num(rest)
                .map(Self::GreaterOrEqual)
                .unwrap_or_else(|_| Self::Invalid(trimmed.to_string()));
        }
        if let Some(rest) = trimmed.strip_prefix("<=") {
            return parse_num(rest)
                .map(Self::LessOrEqual)
                .unwrap_or_else(|_| Self::Invalid(trimmed.to_string()));
        }
        if let Some(rest) = trimmed.strip_prefix("==") {
            return parse_num(rest)
                .map(Self::Exact)
                .unwrap_or_else(|_| Self::Invalid(trimmed.to_string()));
        }
        if let Some(rest) = trimmed.strip_prefix('=') {
            return parse_num(rest)
                .map(Self::Exact)
                .unwrap_or_else(|_| Self::Invalid(trimmed.to_string()));
        }
        if let Some(rest) = trimmed.strip_prefix('>') {
            return parse_num(rest)
                .map(Self::Greater)
                .unwrap_or_else(|_| Self::Invalid(trimmed.to_string()));
        }
        if let Some(rest) = trimmed.strip_prefix('<') {
            return parse_num(rest)
                .map(Self::Less)
                .unwrap_or_else(|_| Self::Invalid(trimmed.to_string()));
        }
        parse_num(trimmed)
            .map(Self::Exact)
            .unwrap_or_else(|_| Self::Invalid(trimmed.to_string()))
    }

    fn matches(&self, hits: u64) -> bool {
        match self {
            HitCondition::Exact(expected) => hits == *expected,
            HitCondition::GreaterOrEqual(expected) => hits >= *expected,
            HitCondition::Greater(expected) => hits > *expected,
            HitCondition::Less(expected) => hits < *expected,
            HitCondition::LessOrEqual(expected) => hits <= *expected,
            HitCondition::Invalid(_) => true,
        }
    }
}

#[derive(Debug, Clone)]
struct BreakpointOptions {
    condition: Option<String>,
    hit_condition: Option<HitCondition>,
    log_message: Option<String>,
}

#[derive(Debug, Clone)]
struct BreakpointHitInfo {
    id: i64,
    condition: Option<String>,
    hit_condition: Option<HitCondition>,
    log_message: Option<String>,
    hit_count: u64,
}

#[derive(Debug, Clone)]
struct ModuleInfo {
    module: Value,
    source: Value,
}

#[derive(Default)]
struct VariablesStore {
    next_ref: i64,
    store: HashMap<i64, Vec<VarItem>>,
}

#[derive(Clone)]
enum WriteMeta {
    Scalar {
        addr: usize,
        kind: ScalarKind,
    },
    Composite {
        addr: usize,
        type_graph: Rc<debugger::ComplexType>,
    },
}

#[derive(Clone, Copy)]
enum ScalarKind {
    I8,
    I16,
    I32,
    I64,
    I128,
    Isize,
    U8,
    U16,
    U32,
    U64,
    U128,
    Usize,
    F32,
    F64,
    Bool,
    Char,
}

#[derive(Clone)]
struct VarItem {
    name: String,
    value: String,
    type_name: Option<String>,
    child: Option<Vec<VarItem>>,
    write: Option<WriteMeta>,
    source: Option<debugger::variable::value::Value>,
}

#[derive(Clone)]
enum DataBreakpointTarget {
    Expr { expression: String, dqe: Dqe },
    Address { addr: usize, size: u8 },
}

#[derive(Clone)]
struct DataBreakpointRecord {
    target: DataBreakpointTarget,
    id: i64,
}

impl DataBreakpointTarget {
    fn data_id(&self) -> String {
        match self {
            DataBreakpointTarget::Expr { expression, .. } => format!("expr:{expression}"),
            DataBreakpointTarget::Address { addr, size } => format!("addr:0x{addr:x}:{size}"),
        }
    }
}

impl VariablesStore {
    fn alloc(&mut self, vars: Vec<VarItem>) -> i64 {
        self.next_ref += 1;
        let key = self.next_ref;
        self.store.insert(key, vars);
        key
    }

    fn get(&self, key: i64) -> Option<&Vec<VarItem>> {
        self.store.get(&key)
    }

    fn get_mut(&mut self, key: i64) -> Option<&mut Vec<VarItem>> {
        self.store.get_mut(&key)
    }

    fn remove(&mut self, key: i64) -> Option<Vec<VarItem>> {
        self.store.remove(&key)
    }

    fn clear(&mut self) {
        self.next_ref = 1;
        self.store.clear();
    }
}

impl DebugSession {
    fn new(io: DapIo) -> Self {
        Self {
            io,
            server_seq: 1,
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
            events: Arc::new(Mutex::new(Vec::new())),
            terminated: false,
            exit_code: None,
            exception_filters: vec![
                EXCEPTION_FILTER_SIGNAL.to_string(),
                EXCEPTION_FILTER_PROCESS.to_string(),
            ],
            last_stop: None,
            module_info: None,
        }
    }

    fn next_seq(&mut self) -> i64 {
        let s = self.server_seq;
        self.server_seq += 1;
        s
    }

    fn exception_filter_for_stop(stop: &debugger::StopReason) -> Option<&'static str> {
        match stop {
            debugger::StopReason::SignalStop(_, _) => Some(EXCEPTION_FILTER_SIGNAL),
            debugger::StopReason::NoSuchProcess(_) => Some(EXCEPTION_FILTER_PROCESS),
            _ => None,
        }
    }

    fn should_stop_on_exception(&self, stop: &debugger::StopReason) -> bool {
        let Some(filter) = Self::exception_filter_for_stop(stop) else {
            return true;
        };
        self.exception_filters.iter().any(|value| value == filter)
    }

    fn enqueue_event(&self, ev: InternalEvent) {
        if let Ok(mut q) = self.events.lock() {
            q.push(ev);
        }
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

    fn active_breakpoint_addresses(&self) -> HashSet<debugger::address::Address> {
        let mut out = HashSet::new();
        for records in self.breakpoints_by_source.values() {
            for record in records {
                out.extend(record.addresses.iter().copied());
            }
        }
        for record in &self.function_breakpoints {
            out.extend(record.addresses.iter().copied());
        }
        for record in &self.instruction_breakpoints {
            out.extend(record.addresses.iter().copied());
        }
        out
    }

    fn parse_breakpoint_options(bp: &Value) -> BreakpointOptions {
        let condition = bp
            .get("condition")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);
        let hit_condition = bp
            .get("hitCondition")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(HitCondition::parse);
        let log_message = bp
            .get("logMessage")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);
        BreakpointOptions {
            condition,
            hit_condition,
            log_message,
        }
    }

    fn with_breakpoint_record_mut<T>(
        &mut self,
        addr: debugger::address::Address,
        f: impl FnOnce(&mut BreakpointRecord) -> T,
    ) -> Option<T> {
        for records in self.breakpoints_by_source.values_mut() {
            if let Some(record) = records
                .iter_mut()
                .find(|record| record.addresses.iter().any(|a| *a == addr))
            {
                return Some(f(record));
            }
        }
        if let Some(record) = self
            .function_breakpoints
            .iter_mut()
            .find(|record| record.addresses.iter().any(|a| *a == addr))
        {
            return Some(f(record));
        }
        if let Some(record) = self
            .instruction_breakpoints
            .iter_mut()
            .find(|record| record.addresses.iter().any(|a| *a == addr))
        {
            return Some(f(record));
        }
        None
    }

    fn record_breakpoint_hit(
        &mut self,
        addr: debugger::address::Address,
    ) -> Option<BreakpointHitInfo> {
        self.with_breakpoint_record_mut(addr, |record| {
            record.hit_count = record.hit_count.saturating_add(1);
            BreakpointHitInfo {
                id: record.id,
                condition: record.condition.clone(),
                hit_condition: record.hit_condition.clone(),
                log_message: record.log_message.clone(),
                hit_count: record.hit_count,
            }
        })
    }

    fn literal_truthy(literal: &Literal) -> bool {
        match literal {
            Literal::String(value) => !value.is_empty(),
            Literal::Int(value) => *value != 0,
            Literal::Float(value) => *value != 0.0,
            Literal::Address(value) => *value != 0,
            Literal::Bool(value) => *value,
            Literal::EnumVariant(_, _) => true,
            Literal::Array(items) => !items.is_empty(),
            Literal::AssocArray(items) => !items.is_empty(),
        }
    }

    fn value_truthy(value: &debugger::variable::value::Value) -> bool {
        if let Some(literal) = value.as_literal() {
            return Self::literal_truthy(&literal);
        }
        let rendered = render_value_to_string(value);
        if rendered == "<unavailable>" {
            return false;
        }
        let trimmed = rendered.trim();
        !(trimmed.is_empty()
            || trimmed == "0"
            || trimmed == "0x0"
            || trimmed == "false"
            || trimmed == "False")
    }

    fn evaluate_condition_expression(&mut self, expr: &str) -> anyhow::Result<bool> {
        let trimmed = expr.trim();
        if trimmed.is_empty() {
            return Ok(true);
        }
        if let Ok(literal) = bs_expr::literal()
            .then_ignore(end())
            .parse(trimmed)
            .into_result()
        {
            return Ok(Self::literal_truthy(&literal));
        }
        let dqe = bs_expr::parser()
            .parse(trimmed)
            .into_result()
            .map_err(|e| anyhow!("condition parse error: {e:?}"))?;
        let dbg = self
            .debugger
            .as_mut()
            .ok_or_else(|| anyhow!("evaluate condition: debugger not initialized"))?;
        let results = dbg
            .read_variable(dqe)
            .context("evaluate condition read_variable")?;
        let Some(result) = results.into_iter().next() else {
            return Ok(false);
        };
        let (_id, value) = result.into_identified_value();
        Ok(Self::value_truthy(&value))
    }

    fn evaluate_expression_string(&mut self, expr: &str) -> anyhow::Result<String> {
        let trimmed = expr.trim();
        if trimmed.is_empty() {
            return Ok(String::new());
        }
        if let Ok(literal) = bs_expr::literal()
            .then_ignore(end())
            .parse(trimmed)
            .into_result()
        {
            return Ok(literal.to_string());
        }
        let dqe = bs_expr::parser()
            .parse(trimmed)
            .into_result()
            .map_err(|e| anyhow!("log point parse error: {e:?}"))?;
        let dbg = self
            .debugger
            .as_mut()
            .ok_or_else(|| anyhow!("log point: debugger not initialized"))?;
        let results = dbg.read_variable(dqe).context("log point read_variable")?;
        if results.is_empty() {
            return Ok("<no result>".to_string());
        }
        let result = results.into_iter().next().unwrap();
        let (_id, val) = result.into_identified_value();
        Ok(render_value_to_string(&val))
    }

    fn format_log_message(&mut self, template: &str) -> anyhow::Result<String> {
        let mut output = String::new();
        let mut chars = template.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '{' {
                let mut expr = String::new();
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next == '}' {
                        break;
                    }
                    expr.push(next);
                }
                if expr.is_empty() {
                    output.push_str("{}");
                } else {
                    match self.evaluate_expression_string(expr.trim()) {
                        Ok(val) => output.push_str(&val),
                        Err(err) => output.push_str(&format!("<error: {err}>")),
                    }
                }
            } else {
                output.push(ch);
            }
        }
        Ok(output)
    }

    fn current_stop_snapshot(&self) -> (Option<String>, Option<i64>, Option<i64>, Option<String>) {
        self.debugger
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
            .unwrap_or((None, None, None, None))
    }

    fn emit_manual_stop(
        &mut self,
        reason: &str,
        description: Option<String>,
    ) -> anyhow::Result<()> {
        self.begin_stop_epoch();
        let _ = self.refresh_threads_with_events();

        let (source_path, line, column, stack_trace) = self.current_stop_snapshot();

        self.last_stop = Some(LastStop {
            reason: reason.to_string(),
            description: description.clone(),
            signal: None,
            source_path,
            line,
            column,
            stack_trace,
        });

        let thread_id = self.current_thread_id();
        self.enqueue_event(InternalEvent::Stopped {
            reason: reason.to_string(),
            thread_id,
            description,
        });
        self.drain_events()
    }

    fn parse_data_breakpoint_expression(expr: &str) -> Result<DataBreakpointTarget, String> {
        let trimmed = expr.trim();
        if trimmed.is_empty() {
            return Err("data breakpoint expression is empty".to_string());
        }

        if let Ok(WatchpointIdentity::Address(addr, size)) = watchpoint_at_address()
            .then_ignore(end())
            .parse(trimmed)
            .into_result()
        {
            return Ok(DataBreakpointTarget::Address { addr, size });
        }

        let dqe = bs_expr::parser()
            .parse(trimmed)
            .into_result()
            .map_err(|e| format!("data breakpoint parse error: {e:?}"))?;
        Ok(DataBreakpointTarget::Expr {
            expression: trimmed.to_string(),
            dqe,
        })
    }

    fn parse_data_breakpoint_id(data_id: &str) -> Result<DataBreakpointTarget, String> {
        let trimmed = data_id.trim();
        if let Some(expr) = trimmed.strip_prefix("expr:") {
            return Self::parse_data_breakpoint_expression(expr);
        }
        if let Some(addr) = trimmed.strip_prefix("addr:") {
            return Self::parse_data_breakpoint_expression(addr);
        }
        Self::parse_data_breakpoint_expression(trimmed)
    }

    fn parse_data_breakpoint_access_type(
        access_type: Option<&str>,
    ) -> Result<BreakCondition, String> {
        match access_type {
            None | Some("write") => Ok(BreakCondition::DataWrites),
            Some("readWrite") => Ok(BreakCondition::DataReadsWrites),
            Some("read") => Err("read accessType is not supported".to_string()),
            Some(other) => Err(format!("unsupported accessType: {other}")),
        }
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

    fn enqueue_thread_event(&self, reason: &'static str, thread_id: i64) {
        self.enqueue_event(InternalEvent::Thread { reason, thread_id });
    }

    fn emit_process_start(&mut self) -> anyhow::Result<()> {
        let dbg = self
            .debugger
            .as_ref()
            .ok_or_else(|| anyhow!("process event: debugger not initialized"))?;
        let process = dbg.process();
        let program = process.program();
        let name = Path::new(program)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(program)
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

    fn refresh_threads_with_events(&mut self) -> anyhow::Result<Vec<Value>> {
        let dbg = self
            .debugger
            .as_ref()
            .ok_or_else(|| anyhow!("threads: debugger not initialized"))?;

        let threads = dbg.thread_state().unwrap_or_default();
        let existing_ids: HashSet<i64> = self.thread_cache.keys().copied().collect();
        let mut new_ids = HashSet::new();
        let mut new_cache = HashMap::new();
        let mut out = Vec::new();
        for t in threads {
            let id = t.thread.pid.as_raw() as i64;
            new_ids.insert(id);
            new_cache.insert(id, t.thread.pid);
            out.push(json!({
                "id": id,
                "name": format!("thread#{} ({})", t.thread.number, t.thread.pid),
            }));
        }

        for id in new_ids.difference(&existing_ids) {
            self.enqueue_thread_event("started", *id);
        }
        for id in existing_ids.difference(&new_ids) {
            self.enqueue_thread_event("exited", *id);
        }
        self.thread_cache = new_cache;
        Ok(out)
    }

    fn drain_events(&mut self) -> anyhow::Result<()> {
        // Drain queued internal events.
        let mut drained = Vec::new();
        if let Ok(mut q) = self.events.lock() {
            drained.append(&mut *q);
        }

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
            // Natural process exit: exited -> terminated (exactly once).
            self.emit_process_end()?;
            self.terminated = true;
            self.exit_code = Some(code);
            self.send_event_body("exited", json!({ "exitCode": code }))?;
            self.send_event("terminated")?;
            return Ok(());
        }

        if has_terminated {
            // User-initiated termination: terminated only (exactly once).
            self.emit_process_end()?;
            self.terminated = true;
            self.send_event("terminated")?;
            return Ok(());
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
        self.io.write_message(&rsp)
    }

    fn send_event(&mut self, name: &'static str) -> anyhow::Result<()> {
        self.send_event_raw(name, None)
    }

    fn send_event_body<T: Serialize>(&mut self, name: &'static str, body: T) -> anyhow::Result<()> {
        let body = serde_json::to_value(body)?;
        self.send_event_raw(name, Some(body))
    }

    fn send_event_raw(&mut self, name: &'static str, body: Option<Value>) -> anyhow::Result<()> {
        let ev = DapEvent {
            seq: self.next_seq(),
            r#type: "event",
            event: name,
            body,
        };
        self.io.write_message(&ev)
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

    fn handle_initialize(&mut self, req: &DapRequest) -> anyhow::Result<()> {
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
                    "filter": EXCEPTION_FILTER_SIGNAL,
                    "label": "Signals",
                    "default": true,
                    "description": "Stop on debuggee signals.",
                },
                {
                    "filter": EXCEPTION_FILTER_PROCESS,
                    "label": "Process",
                    "default": true,
                    "description": "Stop when the debuggee process disappears.",
                },
            ],
        });
        self.send_success_body(req, body)?;
        self.send_event("initialized")
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

        // Source map (remote/WSL/container path mapping).
        self.source_map = SourceMap::from_launch_args(&req.arguments);
        // Reset session termination state for a new launch.
        self.terminated = false;
        self.exit_code = None;
        self.session_mode = Some(SessionMode::Launch);

        self.build_debugger(program, &args, oracles)?;
        self.emit_process_start()?;
        self.send_success(req)?;
        Ok(())
    }

    fn handle_attach(&mut self, req: &DapRequest, oracles: &[String]) -> anyhow::Result<()> {
        let pid = Self::attach_pid(&req.arguments)?;

        self.source_map = SourceMap::from_launch_args(&req.arguments);
        self.terminated = false;
        self.exit_code = None;
        self.session_mode = Some(SessionMode::Attach);

        self.build_attached_debugger(pid, oracles)?;
        self.emit_process_start()?;
        self.send_success(req)?;
        Ok(())
    }

    fn handle_configuration_done(&mut self, req: &DapRequest) -> anyhow::Result<()> {
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

    fn should_skip_breakpoint(&mut self, pid: Pid, addr: RelocatedAddress) -> anyhow::Result<bool> {
        let Some(hit) = self.record_breakpoint_hit(debugger::address::Address::Relocated(addr))
        else {
            return Ok(false);
        };

        {
            let dbg = self
                .debugger
                .as_mut()
                .ok_or_else(|| anyhow!("breakpoint: debugger not initialized"))?;
            let _ = dbg.set_thread_into_focus_by_pid(pid);
        }

        if let Some(condition) = hit.condition.as_deref() {
            match self.evaluate_condition_expression(condition) {
                Ok(false) => return Ok(true),
                Ok(true) => {}
                Err(err) => {
                    let output = format!("Breakpoint {} condition error: {err}\n", hit.id);
                    self.enqueue_event(InternalEvent::Output {
                        category: "console",
                        output,
                    });
                    self.drain_events()?;
                    return Ok(false);
                }
            }
        }

        if let Some(hit_condition) = &hit.hit_condition {
            if let HitCondition::Invalid(raw) = hit_condition {
                let output = format!("Breakpoint {} hitCondition invalid: {raw}\n", hit.id);
                self.enqueue_event(InternalEvent::Output {
                    category: "console",
                    output,
                });
                self.drain_events()?;
            } else if !hit_condition.matches(hit.hit_count) {
                return Ok(true);
            }
        }

        if let Some(log_message) = hit.log_message.as_deref() {
            let mut output = self.format_log_message(log_message)?;
            if !output.ends_with('\n') {
                output.push('\n');
            }
            self.enqueue_event(InternalEvent::Output {
                category: "console",
                output,
            });
            self.drain_events()?;
            return Ok(true);
        }

        Ok(false)
    }

    fn emit_stop_reason(&mut self, stop: debugger::StopReason) -> anyhow::Result<()> {
        let mut stop = stop;
        loop {
            while !self.should_stop_on_exception(&stop) {
                let dbg = self
                    .debugger
                    .as_mut()
                    .ok_or_else(|| anyhow!("continue: debugger not initialized"))?;
                stop = dbg
                    .continue_debugee_with_reason()
                    .context("continue after exception filter")?;
            }

            if let debugger::StopReason::Breakpoint(pid, addr) = stop {
                if self.should_skip_breakpoint(pid, addr)? {
                    let dbg = self
                        .debugger
                        .as_mut()
                        .ok_or_else(|| anyhow!("continue: debugger not initialized"))?;
                    stop = dbg
                        .continue_debugee_with_reason()
                        .context("continue after breakpoint filter")?;
                    continue;
                }
            }
            break;
        }

        let (reason, thread_id, description, exited, last_reason, signal) = match stop {
            debugger::StopReason::DebugeeExit(code) => (
                "exited".to_string(),
                None,
                None,
                Some(code),
                "exited".to_string(),
                None,
            ),
            debugger::StopReason::DebugeeStart => (
                "entry".to_string(),
                self.current_thread_id(),
                Some("Debugee started".to_string()),
                None,
                "entry".to_string(),
                None,
            ),
            debugger::StopReason::Breakpoint(pid, _) => (
                "breakpoint".to_string(),
                Some(pid.as_raw() as i64),
                None,
                None,
                "breakpoint".to_string(),
                None,
            ),
            debugger::StopReason::Watchpoint(pid, _, _) => (
                "data breakpoint".to_string(),
                Some(pid.as_raw() as i64),
                None,
                None,
                "data breakpoint".to_string(),
                None,
            ),
            debugger::StopReason::SignalStop(pid, sign) => (
                "exception".to_string(),
                Some(pid.as_raw() as i64),
                Some(format!("Signal: {sign:?}")),
                None,
                EXCEPTION_FILTER_SIGNAL.to_string(),
                Some(sign as i32),
            ),
            debugger::StopReason::NoSuchProcess(pid) => (
                "exception".to_string(),
                Some(pid.as_raw() as i64),
                Some("No such process".to_string()),
                None,
                EXCEPTION_FILTER_PROCESS.to_string(),
                None,
            ),
        };

        if let Some(code) = exited {
            self.last_stop = None;
            self.enqueue_event(InternalEvent::Exited { code });
            self.drain_events()?;
            return Ok(());
        }

        self.begin_stop_epoch();
        let _ = self.refresh_threads_with_events();

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

        self.last_stop = Some(LastStop {
            reason: last_reason,
            description: description.clone(),
            signal,
            source_path,
            line,
            column,
            stack_trace,
        });

        self.enqueue_event(InternalEvent::Stopped {
            reason,
            thread_id,
            description,
        });
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

        self.last_stop = Some(LastStop {
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

    fn current_thread_id(&mut self) -> Option<i64> {
        self.debugger
            .as_ref()
            .map(|d| d.ecx().pid_on_focus().as_raw() as i64)
    }

    fn refresh_threads(&mut self) -> anyhow::Result<Vec<Value>> {
        self.refresh_threads_with_events()
    }

    fn handle_threads(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let threads = self.refresh_threads()?;
        self.send_success_body(req, json!({"threads": threads}))
    }

    fn handle_set_breakpoints(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let client_source_path = req
            .arguments
            .get("source")
            .and_then(|s| s.get("path"))
            .and_then(|p| p.as_str())
            .ok_or_else(|| anyhow!("setBreakpoints: missing arguments.source.path"))?
            .to_string();

        let source_path = self.source_map.map_client_to_target(&client_source_path);

        let prev = self
            .breakpoints_by_source
            .remove(&source_path)
            .unwrap_or_default();
        let mut new_breakpoints = Vec::new();
        let mut rsp_bps = Vec::new();
        let mut pending_events = Vec::new();
        let bps = req
            .arguments
            .get("breakpoints")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut next_id = self.next_breakpoint_id;
        {
            let dbg = self
                .debugger
                .as_mut()
                .ok_or_else(|| anyhow!("setBreakpoints: debugger not initialized"))?;

            for record in prev {
                for addr in record.addresses {
                    let _ = dbg.remove_breakpoint(addr);
                }
                pending_events.push(InternalEvent::Breakpoint {
                    reason: "removed",
                    breakpoint: json!({ "id": record.id }),
                });
            }

            for bp in bps {
                let line = bp.get("line").and_then(|v| v.as_i64()).unwrap_or(1) as u64;
                let options = Self::parse_breakpoint_options(&bp);
                let mut views = dbg.set_breakpoint_at_line(&source_path, line);
                if views.is_err() {
                    // fallback: try basename, helps when debug info stores only file name
                    if let Some(base) = Path::new(&source_path).file_name().and_then(|s| s.to_str())
                    {
                        views = dbg.set_breakpoint_at_line(base, line);
                    }
                }

                let mut alloc_id = || {
                    let id = next_id;
                    next_id += 1;
                    id
                };

                match views {
                    Ok(mut v) if !v.is_empty() => {
                        let first = v.remove(0);
                        let id = alloc_id();
                        let dap_bp = json!({
                            "id": id,
                            "verified": true,
                            "line": line,
                            "source": { "path": client_source_path },
                        });
                        new_breakpoints.push(BreakpointRecord {
                            id,
                            addresses: vec![first.addr],
                            condition: options.condition,
                            hit_condition: options.hit_condition,
                            log_message: options.log_message,
                            hit_count: 0,
                        });
                        rsp_bps.push(dap_bp.clone());
                        pending_events.push(InternalEvent::Breakpoint {
                            reason: "changed",
                            breakpoint: dap_bp,
                        });
                    }
                    _ => {
                        let id = alloc_id();
                        let dap_bp = json!({
                            "id": id,
                            "verified": false,
                            "line": line,
                            "source": { "path": client_source_path },
                        });
                        new_breakpoints.push(BreakpointRecord {
                            id,
                            addresses: Vec::new(),
                            condition: options.condition,
                            hit_condition: options.hit_condition,
                            log_message: options.log_message,
                            hit_count: 0,
                        });
                        rsp_bps.push(dap_bp.clone());
                        pending_events.push(InternalEvent::Breakpoint {
                            reason: "changed",
                            breakpoint: dap_bp,
                        });
                    }
                }
            }
        }
        self.next_breakpoint_id = next_id;

        self.breakpoints_by_source
            .insert(source_path, new_breakpoints);
        for event in pending_events {
            self.enqueue_event(event);
        }
        self.send_success_body(req, json!({"breakpoints": rsp_bps}))?;
        self.drain_events()
    }

    fn handle_set_function_breakpoints(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let prev = std::mem::take(&mut self.function_breakpoints);
        let bps = req
            .arguments
            .get("breakpoints")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut rsp_bps = Vec::new();
        let mut new_breakpoints = Vec::new();
        let mut pending_events = Vec::new();
        let mut next_id = self.next_breakpoint_id;
        {
            let dbg = self
                .debugger
                .as_mut()
                .ok_or_else(|| anyhow!("setFunctionBreakpoints: debugger not initialized"))?;

            for record in prev {
                for addr in record.addresses {
                    let _ = dbg.remove_breakpoint(addr);
                }
                pending_events.push(InternalEvent::Breakpoint {
                    reason: "removed",
                    breakpoint: json!({ "id": record.id }),
                });
            }

            let mut alloc_id = || {
                let id = next_id;
                next_id += 1;
                id
            };

            for bp in bps {
                let options = Self::parse_breakpoint_options(&bp);
                let Some(name) = bp.get("name").and_then(|v| v.as_str()) else {
                    let id = alloc_id();
                    let dap_bp = json!({
                        "id": id,
                        "verified": false,
                        "message": "setFunctionBreakpoints: missing breakpoint name",
                    });
                    new_breakpoints.push(BreakpointRecord {
                        id,
                        addresses: Vec::new(),
                        condition: options.condition,
                        hit_condition: options.hit_condition,
                        log_message: options.log_message,
                        hit_count: 0,
                    });
                    rsp_bps.push(dap_bp.clone());
                    pending_events.push(InternalEvent::Breakpoint {
                        reason: "changed",
                        breakpoint: dap_bp,
                    });
                    continue;
                };

                match dbg.set_breakpoint_at_fn(name) {
                    Ok(views) if !views.is_empty() => {
                        let id = alloc_id();
                        let dap_bp = json!({
                            "id": id,
                            "verified": true,
                        });
                        new_breakpoints.push(BreakpointRecord {
                            id,
                            addresses: views.iter().map(|view| view.addr).collect(),
                            condition: options.condition,
                            hit_condition: options.hit_condition,
                            log_message: options.log_message,
                            hit_count: 0,
                        });
                        rsp_bps.push(dap_bp.clone());
                        pending_events.push(InternalEvent::Breakpoint {
                            reason: "changed",
                            breakpoint: dap_bp,
                        });
                    }
                    Ok(_) => {
                        let id = alloc_id();
                        let dap_bp = json!({
                            "id": id,
                            "verified": false,
                            "message": "setFunctionBreakpoints: no matching symbols",
                        });
                        new_breakpoints.push(BreakpointRecord {
                            id,
                            addresses: Vec::new(),
                            condition: options.condition,
                            hit_condition: options.hit_condition,
                            log_message: options.log_message,
                            hit_count: 0,
                        });
                        rsp_bps.push(dap_bp.clone());
                        pending_events.push(InternalEvent::Breakpoint {
                            reason: "changed",
                            breakpoint: dap_bp,
                        });
                    }
                    Err(err) => {
                        let id = alloc_id();
                        let dap_bp = json!({
                            "id": id,
                            "verified": false,
                            "message": format!("setFunctionBreakpoints: {err}"),
                        });
                        new_breakpoints.push(BreakpointRecord {
                            id,
                            addresses: Vec::new(),
                            condition: options.condition,
                            hit_condition: options.hit_condition,
                            log_message: options.log_message,
                            hit_count: 0,
                        });
                        rsp_bps.push(dap_bp.clone());
                        pending_events.push(InternalEvent::Breakpoint {
                            reason: "changed",
                            breakpoint: dap_bp,
                        });
                    }
                }
            }
        }
        self.next_breakpoint_id = next_id;
        self.function_breakpoints = new_breakpoints;
        for event in pending_events {
            self.enqueue_event(event);
        }

        self.send_success_body(req, json!({"breakpoints": rsp_bps}))?;
        self.drain_events()
    }

    fn handle_set_instruction_breakpoints(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let prev = std::mem::take(&mut self.instruction_breakpoints);
        let bps = req
            .arguments
            .get("breakpoints")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut rsp_bps = Vec::new();
        let mut new_breakpoints = Vec::new();
        let mut pending_events = Vec::new();
        let mut next_id = self.next_breakpoint_id;
        {
            let dbg = self
                .debugger
                .as_mut()
                .ok_or_else(|| anyhow!("setInstructionBreakpoints: debugger not initialized"))?;

            for record in prev {
                for addr in record.addresses {
                    let _ = dbg.remove_breakpoint(addr);
                }
                pending_events.push(InternalEvent::Breakpoint {
                    reason: "removed",
                    breakpoint: json!({ "id": record.id }),
                });
            }

            let mut alloc_id = || {
                let id = next_id;
                next_id += 1;
                id
            };

            for bp in bps {
                let options = Self::parse_breakpoint_options(&bp);
                let Some(reference) = bp.get("instructionReference").and_then(|v| v.as_str())
                else {
                    let id = alloc_id();
                    let dap_bp = json!({
                        "id": id,
                        "verified": false,
                        "message": "setInstructionBreakpoints: missing instructionReference",
                    });
                    new_breakpoints.push(BreakpointRecord {
                        id,
                        addresses: Vec::new(),
                        condition: options.condition,
                        hit_condition: options.hit_condition,
                        log_message: options.log_message,
                        hit_count: 0,
                    });
                    rsp_bps.push(dap_bp.clone());
                    pending_events.push(InternalEvent::Breakpoint {
                        reason: "changed",
                        breakpoint: dap_bp,
                    });
                    continue;
                };

                let offset = bp.get("offset").and_then(|v| v.as_i64()).unwrap_or(0);
                let addr = match Self::parse_memory_reference_with_offset(reference, offset) {
                    Ok(addr) => addr,
                    Err(err) => {
                        let id = alloc_id();
                        let dap_bp = json!({
                            "id": id,
                            "verified": false,
                            "message": format!("setInstructionBreakpoints: {err}"),
                        });
                        new_breakpoints.push(BreakpointRecord {
                            id,
                            addresses: Vec::new(),
                            condition: options.condition,
                            hit_condition: options.hit_condition,
                            log_message: options.log_message,
                            hit_count: 0,
                        });
                        rsp_bps.push(dap_bp.clone());
                        pending_events.push(InternalEvent::Breakpoint {
                            reason: "changed",
                            breakpoint: dap_bp,
                        });
                        continue;
                    }
                };

                match dbg.set_breakpoint_at_addr(RelocatedAddress::from(addr)) {
                    Ok(view) => {
                        let id = alloc_id();
                        let dap_bp = json!({
                            "id": id,
                            "verified": true,
                            "instructionReference": format!("0x{addr:x}"),
                        });
                        new_breakpoints.push(BreakpointRecord {
                            id,
                            addresses: vec![view.addr],
                            condition: options.condition,
                            hit_condition: options.hit_condition,
                            log_message: options.log_message,
                            hit_count: 0,
                        });
                        rsp_bps.push(dap_bp.clone());
                        pending_events.push(InternalEvent::Breakpoint {
                            reason: "changed",
                            breakpoint: dap_bp,
                        });
                    }
                    Err(err) => {
                        let id = alloc_id();
                        let dap_bp = json!({
                            "id": id,
                            "verified": false,
                            "message": format!("setInstructionBreakpoints: {err}"),
                        });
                        new_breakpoints.push(BreakpointRecord {
                            id,
                            addresses: Vec::new(),
                            condition: options.condition,
                            hit_condition: options.hit_condition,
                            log_message: options.log_message,
                            hit_count: 0,
                        });
                        rsp_bps.push(dap_bp.clone());
                        pending_events.push(InternalEvent::Breakpoint {
                            reason: "changed",
                            breakpoint: dap_bp,
                        });
                    }
                }
            }
        }
        self.next_breakpoint_id = next_id;
        self.instruction_breakpoints = new_breakpoints;
        for event in pending_events {
            self.enqueue_event(event);
        }

        self.send_success_body(req, json!({"breakpoints": rsp_bps}))?;
        self.drain_events()
    }

    fn handle_continue(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        self.begin_running();

        let dbg = self
            .debugger
            .as_mut()
            .ok_or_else(|| anyhow!("continue: debugger not initialized"))?;

        let stop = dbg.continue_debugee_with_reason().context("continue")?;
        let thread_id = self.current_thread_id();
        self.enqueue_event(InternalEvent::Continued {
            thread_id,
            all_threads_continued: true,
        });
        self.send_success_body(req, json!({"allThreadsContinued": true}))?;
        self.emit_stop_reason(stop)
    }

    fn handle_next(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        self.begin_running();

        let dbg = self
            .debugger
            .as_mut()
            .ok_or_else(|| anyhow!("next: debugger not initialized"))?;
        // Blocking step-over.

        match dbg.step_over() {
            Ok(()) => {
                let thread_id = self.current_thread_id();
                self.enqueue_event(InternalEvent::Continued {
                    thread_id,
                    all_threads_continued: true,
                });
                self.send_success_body(req, json!({"allThreadsContinued": true}))?;
                self.begin_stop_epoch();

                self.last_stop = Some(LastStop {
                    reason: "pause".to_string(),
                    description: Some("Paused".to_string()),
                    signal: None,
                    source_path: None,
                    line: None,
                    column: None,
                    stack_trace: None,
                });

                let thread_id = self.current_thread_id();
                self.enqueue_event(InternalEvent::Stopped {
                    reason: "step".to_string(),
                    thread_id,
                    description: None,
                });
                self.drain_events()
            }
            Err(debugger::Error::ProcessExit(code)) => {
                let thread_id = self.current_thread_id();
                self.enqueue_event(InternalEvent::Continued {
                    thread_id,
                    all_threads_continued: true,
                });
                self.send_success_body(req, json!({"allThreadsContinued": true}))?;
                self.enqueue_event(InternalEvent::Exited { code });
                self.drain_events()
            }
            Err(e) => self.send_err(req, format!("next failed: {e}")),
        }
    }

    fn handle_step_in(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        self.begin_running();

        let dbg = self
            .debugger
            .as_mut()
            .ok_or_else(|| anyhow!("stepIn: debugger not initialized"))?;

        match dbg.step_into() {
            Ok(()) => {
                let thread_id = self.current_thread_id();
                self.enqueue_event(InternalEvent::Continued {
                    thread_id,
                    all_threads_continued: true,
                });
                self.send_success_body(req, json!({"allThreadsContinued": true}))?;
                self.begin_stop_epoch();

                let thread_id = self.current_thread_id();
                self.enqueue_event(InternalEvent::Stopped {
                    reason: "step".to_string(),
                    thread_id,
                    description: None,
                });
                self.drain_events()
            }
            Err(debugger::Error::ProcessExit(code)) => {
                let thread_id = self.current_thread_id();
                self.enqueue_event(InternalEvent::Continued {
                    thread_id,
                    all_threads_continued: true,
                });
                self.send_success_body(req, json!({"allThreadsContinued": true}))?;
                self.enqueue_event(InternalEvent::Exited { code });
                self.drain_events()
            }
            Err(e) => self.send_err(req, format!("stepIn failed: {e}")),
        }
    }

    fn handle_step_out(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        self.begin_running();

        let dbg = self
            .debugger
            .as_mut()
            .ok_or_else(|| anyhow!("stepOut: debugger not initialized"))?;

        match dbg.step_out() {
            Ok(()) => {
                let thread_id = self.current_thread_id();
                self.enqueue_event(InternalEvent::Continued {
                    thread_id,
                    all_threads_continued: true,
                });
                self.send_success_body(req, json!({"allThreadsContinued": true}))?;
                self.begin_stop_epoch();

                let thread_id = self.current_thread_id();
                self.enqueue_event(InternalEvent::Stopped {
                    reason: "step".to_string(),
                    thread_id,
                    description: None,
                });
                self.drain_events()
            }
            Err(debugger::Error::ProcessExit(code)) => {
                let thread_id = self.current_thread_id();
                self.enqueue_event(InternalEvent::Continued {
                    thread_id,
                    all_threads_continued: true,
                });
                self.send_success_body(req, json!({"allThreadsContinued": true}))?;
                self.enqueue_event(InternalEvent::Exited { code });
                self.drain_events()
            }
            Err(e) => self.send_err(req, format!("stepOut failed: {e}")),
        }
    }

    fn handle_pause(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let dbg = match self.debugger.as_mut() {
            None => {
                self.send_err(req, "no active debug session")?;
                return Ok(());
            }
            Some(dbg) => dbg,
        };

        // Pause should stop the debugee and then emit a `stopped` event.
        match dbg.pause_debugee() {
            Ok(()) => {
                self.send_success(req)?;

                self.begin_stop_epoch();

                let thread_id = self.current_thread_id();
                self.enqueue_event(InternalEvent::Stopped {
                    reason: "pause".to_string(),
                    thread_id,
                    description: Some("Paused".to_string()),
                });
            }
            Err(e) => {
                self.send_err(req, format!("pause failed: {e}"))?;
            }
        }

        Ok(())
    }

    fn handle_restart(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        if self.session_mode != Some(SessionMode::Launch) {
            return self.send_err(req, "restart is only supported for launch sessions");
        }

        self.begin_running();

        let dbg = self
            .debugger
            .as_mut()
            .ok_or_else(|| anyhow!("restart: debugger not initialized"))?;

        let stop = dbg
            .start_debugee_force_with_reason()
            .context("restart debugee")?;
        self.send_success(req)?;
        self.emit_stop_reason(stop)
    }

    fn handle_restart_frame(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let dbg = self
            .debugger
            .as_mut()
            .ok_or_else(|| anyhow!("restartFrame: debugger not initialized"))?;

        let frame_id = req
            .arguments
            .get("frameId")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow!("restartFrame: missing arguments.frameId"))?;
        let (thread_id, frame_num) = Self::decode_frame_id(frame_id);
        if frame_num != 0 {
            return self.send_err(req, "restartFrame: only the top frame (0) can be restarted");
        }

        let pid = self
            .thread_cache
            .get(&thread_id)
            .copied()
            .unwrap_or_else(|| Pid::from_raw(thread_id as i32));
        let _ = dbg.set_thread_into_focus_by_pid(pid);

        let bt = dbg.backtrace(pid).unwrap_or_default();
        let frame = bt
            .get(frame_num as usize)
            .ok_or_else(|| anyhow!("restartFrame: frame {frame_num} not found"))?;
        let Some(start_ip) = frame.fn_start_ip else {
            return self.send_err(req, "restartFrame: function start address is unavailable");
        };

        dbg.set_register_value("rip", start_ip.as_u64())
            .context("restartFrame: set rip")?;
        let _ = dbg.set_frame_into_focus(0);

        self.send_success(req)?;
        self.emit_manual_stop("restart", None)
    }

    fn handle_goto_targets(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let args = req
            .arguments
            .as_object()
            .ok_or_else(|| anyhow!("gotoTargets: arguments must be object"))?;
        let source_path = args
            .get("source")
            .and_then(|v| v.get("path"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("gotoTargets: missing arguments.source.path"))?;
        let line = args
            .get("line")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow!("gotoTargets: missing arguments.line"))?;
        let column = args.get("column").and_then(|v| v.as_i64()).unwrap_or(1);

        let target_path = self.source_map.map_client_to_target(source_path);
        let existing_breakpoints = self.active_breakpoint_addresses();
        let dbg = self
            .debugger
            .as_mut()
            .ok_or_else(|| anyhow!("gotoTargets: debugger not initialized"))?;
        let mut views = dbg.set_breakpoint_at_line(&target_path, line as u64);
        if views.is_err()
            && let Some(base) = Path::new(&target_path).file_name().and_then(|s| s.to_str())
        {
            views = dbg.set_breakpoint_at_line(base, line as u64);
        }

        let (targets, remove_addrs) = match views {
            Ok(views) => {
                let mut targets = Vec::new();
                let mut remove_addrs = Vec::new();
                for view in &views {
                    let relocated = match view.addr {
                        debugger::address::Address::Relocated(addr) => addr,
                        debugger::address::Address::Global(addr) => {
                            debugger::address::RelocatedAddress::from(u64::from(addr))
                        }
                    };
                    let address = relocated.as_u64();
                    let target_id = i64::try_from(address)
                        .map_err(|_| anyhow!("gotoTargets: target address out of range"))?;

                    let (line_out, col_out, label) = if let Some(place) = view.place.as_ref() {
                        let mapped = self
                            .source_map
                            .map_target_to_client(place.file.to_string_lossy().as_ref());
                        (
                            place.line_number as i64,
                            place.column_number as i64,
                            format!("{mapped}:{}", place.line_number),
                        )
                    } else {
                        (line, column, format!("0x{address:x}"))
                    };

                    targets.push(json!({
                        "id": target_id,
                        "label": label,
                        "line": line_out,
                        "column": col_out,
                        "instructionPointerReference": format!("0x{address:x}"),
                    }));
                }

                for view in &views {
                    if !existing_breakpoints.contains(&view.addr) {
                        remove_addrs.push(view.addr);
                    }
                }

                (targets, remove_addrs)
            }
            Err(_) => (Vec::new(), Vec::new()),
        };

        for addr in remove_addrs {
            let _ = dbg.remove_breakpoint(addr);
        }

        self.send_success_body(req, json!({ "targets": targets }))
    }

    fn handle_goto(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let dbg = self
            .debugger
            .as_mut()
            .ok_or_else(|| anyhow!("goto: debugger not initialized"))?;

        let args = req
            .arguments
            .as_object()
            .ok_or_else(|| anyhow!("goto: arguments must be object"))?;
        let addr = if let Some(target_id) = args.get("targetId").and_then(|v| v.as_i64()) {
            let addr = u64::try_from(target_id).map_err(|_| anyhow!("goto: targetId invalid"))?;
            addr as usize
        } else if let Some(reference) = args.get("instructionReference").and_then(|v| v.as_str()) {
            Self::parse_memory_reference_with_offset(reference, 0)?
        } else {
            return self.send_err(req, "goto: missing arguments.targetId");
        };

        if let Some(thread_id) = args.get("threadId").and_then(|v| v.as_i64()) {
            let pid = self
                .thread_cache
                .get(&thread_id)
                .copied()
                .unwrap_or_else(|| Pid::from_raw(thread_id as i32));
            let _ = dbg.set_thread_into_focus_by_pid(pid);
        }

        dbg.set_register_value("rip", addr as u64)
            .context("goto: set rip")?;
        let _ = dbg.set_frame_into_focus(0);

        self.send_success(req)?;
        self.emit_manual_stop("goto", None)
    }

    fn handle_step_back(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        self.send_err(
            req,
            "stepBack: reverse execution is not supported by the current engine",
        )
    }

    fn handle_reverse_continue(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        self.send_err(
            req,
            "reverseContinue: reverse execution is not supported by the current engine",
        )
    }

    fn handle_completions(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let args = req
            .arguments
            .as_object()
            .ok_or_else(|| anyhow!("completions: arguments must be object (possibly empty)"))?;
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let column = args.get("column").and_then(|v| v.as_i64());
        let (prefix, start_column, prefix_len) = completion_prefix(text, column);

        let mut targets = Vec::new();
        let mut seen = HashSet::new();

        let Some(dbg) = self.debugger.as_mut() else {
            return self.send_success_body(req, json!({ "targets": targets }));
        };

        if let Some(frame_id) = args.get("frameId").and_then(|v| v.as_i64()) {
            let (thread_id, frame_num) = Self::decode_frame_id(frame_id);
            let pid = self
                .thread_cache
                .get(&thread_id)
                .copied()
                .unwrap_or_else(|| Pid::from_raw(thread_id as i32));
            let _ = dbg.set_thread_into_focus_by_pid(pid);
            let _ = dbg.set_frame_into_focus(frame_num);
        }

        let mut push_item = |label: String| {
            if !seen.insert(label.clone()) {
                return;
            }
            let mut item = json!({
                "label": label,
            });
            if prefix_len > 0 {
                item["text"] = item["label"].clone();
                item["start"] = json!(start_column);
                item["length"] = json!(prefix_len);
            }
            targets.push(item);
        };

        if let Ok(locals) = read_locals(dbg) {
            for v in locals {
                if prefix.is_empty() || v.name.starts_with(&prefix) {
                    push_item(v.name);
                }
            }
        }

        if let Ok(args_vars) = read_args(dbg) {
            for v in args_vars {
                if prefix.is_empty() || v.name.starts_with(&prefix) {
                    push_item(v.name);
                }
            }
        }

        if !prefix.is_empty() {
            let regex = format!("^{}", regex_escape(&prefix));
            if let Ok(symbols) = dbg.get_symbols(&regex) {
                for sym in symbols {
                    push_item(sym.name.to_string());
                }
            }
        }

        self.send_success_body(req, json!({ "targets": targets }))
    }

    fn handle_loaded_sources(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let sources = self
            .module_info
            .as_ref()
            .map(|info| vec![info.source.clone()])
            .unwrap_or_default();
        self.send_success_body(req, json!({ "sources": sources }))
    }

    fn handle_modules(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let modules = self
            .module_info
            .as_ref()
            .map(|info| vec![info.module.clone()])
            .unwrap_or_default();
        let total = modules.len();
        self.send_success_body(req, json!({ "modules": modules, "totalModules": total }))
    }

    fn handle_read_memory(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let dbg = self
            .debugger
            .as_ref()
            .ok_or_else(|| anyhow!("readMemory: debugger not initialized"))?;
        let args = req
            .arguments
            .as_object()
            .ok_or_else(|| anyhow!("readMemory: arguments must be object"))?;

        let memory_reference = args
            .get("memoryReference")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow!("readMemory: missing arguments.memoryReference"))?;
        let count = args
            .get("count")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| anyhow!("readMemory: missing arguments.count"))?;
        if count < 0 {
            return self.send_err(req, "readMemory: count must be non-negative");
        }

        let offset = args
            .get("offset")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);

        let addr = Self::parse_memory_reference_with_offset(memory_reference, offset)
            .context("readMemory: invalid memoryReference")?;
        let bytes = dbg
            .read_memory(addr, count as usize)
            .context("readMemory: read_memory")?;
        let data = BASE64_ENGINE.encode(bytes);
        self.send_success_body(
            req,
            json!({
                "address": format!("0x{addr:x}"),
                "data": data,
            }),
        )
    }

    fn handle_write_memory(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let dbg = self
            .debugger
            .as_ref()
            .ok_or_else(|| anyhow!("writeMemory: debugger not initialized"))?;
        let args = req
            .arguments
            .as_object()
            .ok_or_else(|| anyhow!("writeMemory: arguments must be object"))?;

        let memory_reference = args
            .get("memoryReference")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow!("writeMemory: missing arguments.memoryReference"))?;
        let data = args
            .get("data")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow!("writeMemory: missing arguments.data"))?;
        let offset = args
            .get("offset")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);

        let addr = Self::parse_memory_reference_with_offset(memory_reference, offset)
            .context("writeMemory: invalid memoryReference")?;
        let bytes = BASE64_ENGINE
            .decode(data)
            .map_err(|err| anyhow!("writeMemory: base64 decode failed: {err}"))?;
        write_bytes(dbg, addr, &bytes).context("writeMemory: write_bytes")?;
        self.send_success_body(req, json!({ "bytesWritten": bytes.len() }))
    }

    fn handle_set_expression(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let dbg = self
            .debugger
            .as_mut()
            .ok_or_else(|| anyhow!("setExpression: debugger not initialized"))?;

        let expression = req
            .arguments
            .get("expression")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("setExpression: missing arguments.expression"))?;
        let new_value = req
            .arguments
            .get("value")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("setExpression: missing arguments.value"))?;

        if let Some(frame_id) = req.arguments.get("frameId").and_then(|v| v.as_i64()) {
            let (thread_id, frame_num) = Self::decode_frame_id(frame_id);
            let pid = self
                .thread_cache
                .get(&thread_id)
                .copied()
                .unwrap_or_else(|| Pid::from_raw(thread_id as i32));
            let _ = dbg.set_thread_into_focus_by_pid(pid);
            let _ = dbg.set_frame_into_focus(frame_num);
        }

        let dqe = bs_expr::parser()
            .parse(expression)
            .into_result()
            .map_err(|e| anyhow!("setExpression parse error: {e:?}"))?;
        let results = dbg
            .read_variable(dqe.clone())
            .context("setExpression read_variable")?;
        let result = results
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("setExpression: expression produced no results"))?;
        let type_graph = Rc::new(result.type_graph().clone());
        let (_id, value) = result.into_identified_value();

        let Some(write_meta) = value_write_meta(&value, type_graph.clone()) else {
            return self.send_err(req, "setExpression: expression is not writable");
        };

        match write_meta {
            WriteMeta::Scalar { addr, kind } => {
                let bytes = parse_set_value(kind, new_value)?;
                write_bytes(dbg, addr, &bytes)?;
            }
            WriteMeta::Composite { addr, type_graph } => {
                let Some(type_id) = value.type_id() else {
                    return self.send_err(req, "setExpression: expression has no type id");
                };
                let serialized = debugger::variable::value::serialize::serialize_dap_value(
                    new_value,
                    &type_graph,
                    type_id,
                    Some(&value),
                )
                .map_err(|err| anyhow!("setExpression: {err}"))?;
                write_bytes(dbg, addr, &serialized.bytes)?;
            }
        }

        let refreshed = dbg
            .read_variable(dqe)
            .context("setExpression read_variable (refresh)")?;
        let response = if let Some(updated) = refreshed.into_iter().next() {
            let type_graph = Rc::new(updated.type_graph().clone());
            let (_id, val) = updated.into_identified_value();
            let child = value_children(&val, type_graph);
            let vars_ref = child.map(|c| self.vars.alloc(c)).unwrap_or(0);
            json!({
                "value": render_value_to_string(&val),
                "type": val.r#type().name_fmt(),
                "variablesReference": vars_ref,
            })
        } else {
            json!({
                "value": new_value,
                "variablesReference": 0,
            })
        };

        self.send_success_body(req, response)
    }

    fn handle_step_in_targets(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let dbg = self
            .debugger
            .as_mut()
            .ok_or_else(|| anyhow!("stepInTargets: debugger not initialized"))?;
        let args = req
            .arguments
            .as_object()
            .ok_or_else(|| anyhow!("stepInTargets: arguments must be object"))?;

        if let Some(frame_id) = args.get("frameId").and_then(|v| v.as_i64()) {
            let (thread_id, frame_num) = Self::decode_frame_id(frame_id);
            let pid = self
                .thread_cache
                .get(&thread_id)
                .copied()
                .unwrap_or_else(|| Pid::from_raw(thread_id as i32));
            let _ = dbg.set_thread_into_focus_by_pid(pid);
            let _ = dbg.set_frame_into_focus(frame_num);
        }

        let asm = dbg.disasm().context("stepInTargets: disasm")?;
        let mut targets = Vec::new();
        let mut seen = HashSet::new();

        for ins in asm.instructions {
            let mnemonic = ins.mnemonic.as_deref().unwrap_or("");
            if !mnemonic.starts_with("call") {
                continue;
            }
            let operands = ins.operands.as_deref().unwrap_or("");
            let Some(target_addr) = parse_call_target_addr(operands) else {
                continue;
            };
            if !seen.insert(target_addr) {
                continue;
            }

            let mut label = format!("0x{target_addr:x}");
            let mut line = None;
            let mut column = None;

            if let Ok(Some((name, place))) =
                dbg.resolve_function_at_pc(GlobalAddress::from(target_addr))
            {
                label = name;
                if let Some(place) = place {
                    line = Some(place.line_number as i64);
                    column = Some(place.column_number as i64);
                }
            }

            let id = i64::try_from(target_addr).unwrap_or(i64::MAX);
            let mut target = json!({ "id": id, "label": label });
            if let Some(line) = line {
                target["line"] = json!(line);
            }
            if let Some(column) = column {
                target["column"] = json!(column);
            }
            targets.push(target);
        }

        self.send_success_body(req, json!({ "targets": targets }))
    }

    fn handle_breakpoint_locations(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let args = req
            .arguments
            .as_object()
            .ok_or_else(|| anyhow!("breakpointLocations: arguments must be object"))?;

        let dbg = self
            .debugger
            .as_ref()
            .ok_or_else(|| anyhow!("breakpointLocations: debugger not initialized"))?;

        if let Some(source) = args.get("source") {
            let source_path = source
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("breakpointLocations: missing source.path"))?;
            let line = args
                .get("line")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| anyhow!("breakpointLocations: missing line"))?;
            let end_line = args.get("endLine").and_then(|v| v.as_i64()).unwrap_or(line);

            let column = args.get("column").and_then(|v| v.as_i64());
            let end_column = args.get("endColumn").and_then(|v| v.as_i64()).or(column);

            let target_path = self.source_map.map_client_to_target(source_path);
            let mut places =
                dbg.breakpoint_places_for_file_range(&target_path, line as u64, end_line as u64)?;
            if places.is_empty()
                && let Some(base) = Path::new(&target_path).file_name().and_then(|s| s.to_str())
            {
                places =
                    dbg.breakpoint_places_for_file_range(base, line as u64, end_line as u64)?;
            }

            let mut breakpoints = Vec::new();
            let mut seen = HashSet::new();
            for place in places {
                let place_column = place.column_number as i64;
                if let Some(column) = column {
                    let end_column = end_column.unwrap_or(column);
                    if place_column < column || place_column > end_column {
                        continue;
                    }
                }
                let key = (place.line_number, place.column_number, place.address);
                if !seen.insert(key) {
                    continue;
                }
                breakpoints.push(json!({
                    "line": place.line_number as i64,
                    "column": place.column_number as i64,
                }));
            }

            return self.send_success_body(req, json!({ "breakpoints": breakpoints }));
        }

        if let Some(reference) = args.get("instructionReference").and_then(|v| v.as_str()) {
            let offset = args.get("offset").and_then(|v| v.as_i64()).unwrap_or(0);
            let end_offset = args.get("endOffset").and_then(|v| v.as_i64());
            let start_addr = Self::parse_memory_reference_with_offset(reference, offset)?;

            let mut breakpoints = Vec::new();
            let mut seen = HashSet::new();
            if let Some(end_offset) = end_offset {
                let end_addr = Self::parse_memory_reference_with_offset(reference, end_offset)?;
                if end_addr < start_addr {
                    return self.send_err(req, "breakpointLocations: endOffset is before offset");
                }
                let end_exclusive = end_addr.saturating_add(1);
                let instructions = disassemble_from_range(dbg, start_addr, end_exclusive)?;
                for ins in instructions {
                    if !seen.insert(ins.address) {
                        continue;
                    }
                    breakpoints.push(json!({
                        "instructionReference": format!("0x{:x}", ins.address),
                    }));
                }
            } else {
                let instructions = disassemble_from_address(dbg, start_addr, 1)?;
                for ins in instructions {
                    if !seen.insert(ins.address) {
                        continue;
                    }
                    breakpoints.push(json!({
                        "instructionReference": format!("0x{:x}", ins.address),
                    }));
                }
            }

            return self.send_success_body(req, json!({ "breakpoints": breakpoints }));
        }

        self.send_err(
            req,
            "breakpointLocations: missing source or instructionReference",
        )
    }

    fn handle_terminate_threads(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let args = req
            .arguments
            .as_object()
            .ok_or_else(|| anyhow!("terminateThreads: arguments must be object"))?;
        let thread_ids = args
            .get("threadIds")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        if thread_ids.is_empty() {
            self.send_success(req)?;
            self.terminate_debuggee();
            self.drain_events()?;
            return Ok(());
        }

        for thread_id in thread_ids {
            let Some(thread_id) = thread_id.as_i64() else {
                return self.send_err(req, "terminateThreads: threadIds must be integers");
            };
            let pid = Pid::from_raw(thread_id as i32);
            signal::kill(pid, Signal::SIGTERM)
                .map_err(|err| anyhow!("terminateThreads: failed to signal {pid}: {err}"))?;
        }

        self.send_success(req)?;
        let _ = self.refresh_threads_with_events();
        self.drain_events()
    }

    fn handle_cancel(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        self.send_success(req)
    }

    fn handle_run_in_terminal(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let args = req
            .arguments
            .as_object()
            .ok_or_else(|| anyhow!("runInTerminal: arguments must be object"))?;
        let argv = args
            .get("args")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow!("runInTerminal: missing arguments.args"))?;
        if argv.is_empty() {
            return self.send_err(req, "runInTerminal: args must not be empty");
        }

        let program = argv
            .first()
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("runInTerminal: args[0] must be a string"))?;
        let mut command = Command::new(program);
        let mut iter = argv.iter();
        iter.next();
        for arg in iter {
            let Some(arg) = arg.as_str() else {
                return self.send_err(req, "runInTerminal: args must be strings");
            };
            command.arg(arg);
        }

        if let Some(cwd) = args.get("cwd").and_then(|v| v.as_str()) {
            command.current_dir(cwd);
        }
        if let Some(env) = args.get("env").and_then(|v| v.as_object()) {
            for (key, value) in env {
                let Some(value) = value.as_str() else {
                    return self.send_err(req, "runInTerminal: env values must be strings");
                };
                command.env(key, value);
            }
        }

        command
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        let child = command.spawn().context("runInTerminal: spawn failed")?;
        let pid = child.id();

        self.send_success_body(
            req,
            json!({
                "processId": pid,
            }),
        )
    }

    fn handle_set_exception_breakpoints(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let filters = req
            .arguments
            .get("filters")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|it| it.as_str().map(|s| s.to_string()))
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();

        self.exception_filters = filters;
        self.send_success(req)
    }

    fn handle_exception_info(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        // The DAP spec allows exceptionInfo to be requested for the currently stopped thread.
        // We expose whatever we know about the last stop reason.
        let Some(last) = self.last_stop.clone() else {
            return self.send_err(req, "exceptionInfo: no stopped state");
        };

        let mut description = last.description.clone();
        if description.is_none() {
            description = Some(last.reason.clone());
        }

        let exception_id = if let Some(sig) = last.signal {
            format!("signal {}", sig)
        } else {
            last.reason.clone()
        };

        let mut details = serde_json::Map::new();
        if let Some(message) = last.description.clone() {
            details.insert("message".to_string(), json!(message));
        }
        if let Some(source_path) = last.source_path.clone() {
            details.insert("source".to_string(), json!({ "path": source_path }));
            if let Some(line) = last.line {
                details.insert("line".to_string(), json!(line));
            }
            if let Some(column) = last.column {
                details.insert("column".to_string(), json!(column));
            }
            details.insert(
                "stackTrace".to_string(),
                json!(
                    last.stack_trace
                        .clone()
                        .unwrap_or_else(|| "<unavailable>".to_string())
                ),
            );
        } else {
            details.insert(
                "message".to_string(),
                json!("Source information unavailable"),
            );
            details.insert("stackTrace".to_string(), json!("<unavailable>"));
        }

        let body = json!({
            "exceptionId": exception_id,
            "description": description,
            "breakMode": "always",
            "details": details,
        });

        self.send_success_body(req, body)
    }

    fn handle_evaluate(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let dbg = self
            .debugger
            .as_mut()
            .ok_or_else(|| anyhow!("evaluate: debugger not initialized"))?;

        let expression = req
            .arguments
            .get("expression")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("evaluate: missing arguments.expression"))?;

        // Optional frameId: if provided, focus thread/frame so evaluation is stable.
        if let Some(frame_id) = req.arguments.get("frameId").and_then(|v| v.as_i64()) {
            let (thread_id, frame_num) = Self::decode_frame_id(frame_id);
            let pid = self
                .thread_cache
                .get(&thread_id)
                .copied()
                .unwrap_or_else(|| Pid::from_raw(thread_id as i32));
            let _ = dbg.set_thread_into_focus_by_pid(pid);
            let _ = dbg.set_frame_into_focus(frame_num);
        }

        let dqe = bs_expr::parser()
            .parse(expression)
            .into_result()
            .map_err(|e| anyhow!("evaluate parse error: {e:?}"))?;

        let results = dbg.read_variable(dqe).context("evaluate read_variable")?;
        if results.is_empty() {
            return self.send_success_body(
                req,
                json!({"result": "<no result>", "variablesReference": 0}),
            );
        }
        let result = results.into_iter().next().unwrap();
        let type_graph = Rc::new(result.type_graph().clone());
        let (_id, val) = result.into_identified_value();
        let child = value_children(&val, type_graph);
        let vars_ref = child.map(|c| self.vars.alloc(c)).unwrap_or(0);
        let result_str = render_value_to_string(&val);
        self.send_success_body(
            req,
            json!({"result": result_str, "variablesReference": vars_ref}),
        )
    }

    fn handle_data_breakpoint_info(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let expression = req
            .arguments
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("dataBreakpointInfo: missing arguments.name"))?;

        match Self::parse_data_breakpoint_expression(expression) {
            Ok(target) => {
                let data_id = target.data_id();
                self.send_success_body(
                    req,
                    json!({
                        "dataId": data_id,
                        "description": expression,
                        "accessTypes": ["write", "readWrite"],
                        "canPersist": false,
                    }),
                )
            }
            Err(err) => self.send_success_body(
                req,
                json!({
                    "dataId": Value::Null,
                    "description": err,
                    "accessTypes": [],
                    "canPersist": false,
                }),
            ),
        }
    }

    fn handle_set_data_breakpoints(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let bps = req
            .arguments
            .get("breakpoints")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let existing = std::mem::take(&mut self.data_breakpoints);
        let mut new_map = HashMap::new();
        let mut rsp_bps = Vec::new();
        let mut pending_events = Vec::new();
        let mut next_id = self.next_breakpoint_id;
        {
            let dbg = self
                .debugger
                .as_mut()
                .ok_or_else(|| anyhow!("setDataBreakpoints: debugger not initialized"))?;

            for (_, record) in existing {
                match record.target {
                    DataBreakpointTarget::Expr { dqe, .. } => {
                        let _ = dbg.remove_watchpoint_by_expr(dqe);
                    }
                    DataBreakpointTarget::Address { addr, .. } => {
                        let _ = dbg.remove_watchpoint_by_addr(RelocatedAddress::from(addr));
                    }
                }
                pending_events.push(InternalEvent::Breakpoint {
                    reason: "removed",
                    breakpoint: json!({ "id": record.id }),
                });
            }

            let mut alloc_id = || {
                let id = next_id;
                next_id += 1;
                id
            };

            for bp in bps {
                let Some(data_id) = bp.get("dataId").and_then(|v| v.as_str()) else {
                    let id = alloc_id();
                    let dap_bp = json!({
                        "id": id,
                        "verified": false,
                        "message": "setDataBreakpoints: missing dataId",
                    });
                    rsp_bps.push(dap_bp.clone());
                    pending_events.push(InternalEvent::Breakpoint {
                        reason: "changed",
                        breakpoint: dap_bp,
                    });
                    continue;
                };

                let condition = match Self::parse_data_breakpoint_access_type(
                    bp.get("accessType").and_then(|v| v.as_str()),
                ) {
                    Ok(cond) => cond,
                    Err(err) => {
                        let id = alloc_id();
                        let dap_bp = json!({
                            "id": id,
                            "verified": false,
                            "message": format!("setDataBreakpoints: {err}"),
                        });
                        rsp_bps.push(dap_bp.clone());
                        pending_events.push(InternalEvent::Breakpoint {
                            reason: "changed",
                            breakpoint: dap_bp,
                        });
                        continue;
                    }
                };

                let target = match Self::parse_data_breakpoint_id(data_id) {
                    Ok(target) => target,
                    Err(err) => {
                        let id = alloc_id();
                        let dap_bp = json!({
                            "id": id,
                            "verified": false,
                            "message": format!("setDataBreakpoints: {err}"),
                        });
                        rsp_bps.push(dap_bp.clone());
                        pending_events.push(InternalEvent::Breakpoint {
                            reason: "changed",
                            breakpoint: dap_bp,
                        });
                        continue;
                    }
                };

                let result: anyhow::Result<()> = (|| match &target {
                    DataBreakpointTarget::Expr { expression, dqe } => dbg
                        .set_watchpoint_on_expr(expression, dqe.clone(), condition)
                        .map(|_| ())
                        .map_err(|err| anyhow!("setDataBreakpoints: {err}")),
                    DataBreakpointTarget::Address { addr, size } => {
                        let break_size = BreakSize::try_from(*size)
                            .map_err(|err| anyhow!("setDataBreakpoints: {err}"))?;
                        dbg.set_watchpoint_on_memory(
                            RelocatedAddress::from(*addr),
                            break_size,
                            condition,
                            false,
                        )
                        .map(|_| ())
                        .map_err(|err| anyhow!("setDataBreakpoints: {err}"))
                    }
                })();

                match result {
                    Ok(()) => {
                        let id = alloc_id();
                        let dap_bp = json!({
                            "id": id,
                            "verified": true,
                        });
                        new_map.insert(data_id.to_string(), DataBreakpointRecord { target, id });
                        rsp_bps.push(dap_bp.clone());
                        pending_events.push(InternalEvent::Breakpoint {
                            reason: "changed",
                            breakpoint: dap_bp,
                        });
                    }
                    Err(err) => {
                        let id = alloc_id();
                        let dap_bp = json!({
                            "id": id,
                            "verified": false,
                            "message": format!("{err}"),
                        });
                        rsp_bps.push(dap_bp.clone());
                        pending_events.push(InternalEvent::Breakpoint {
                            reason: "changed",
                            breakpoint: dap_bp,
                        });
                    }
                }
            }
        }
        self.next_breakpoint_id = next_id;

        self.data_breakpoints = new_map;
        for event in pending_events {
            self.enqueue_event(event);
        }
        self.send_success_body(req, json!({"breakpoints": rsp_bps}))?;
        self.drain_events()
    }

    fn handle_stack_trace(&mut self, req: &DapRequest) -> anyhow::Result<()> {
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

        let bt = self
            .debugger
            .as_ref()
            .ok_or_else(|| anyhow!("stackTrace: debugger not initialized"))?
            .backtrace(pid)
            .unwrap_or_default();
        let mut frames = Vec::new();
        for (i, f) in bt.iter().enumerate() {
            let (path, line, col, source_reference) = match f.place.as_ref() {
                Some(p) => (
                    Some(p.file.to_string_lossy().to_string()),
                    Some(p.line_number as i64),
                    Some(p.column_number as i64),
                    None,
                ),
                None => {
                    let addr = f.ip.as_usize();
                    let disasm = self.disasm_source_for_address(addr)?;
                    (None, Some(1), Some(1), Some(disasm.reference))
                }
            };
            let name = f.func_name.as_deref().unwrap_or("<unknown>").to_string();
            let frame_id = (thread_id << 16) | (i as i64);
            let source = if let Some(path) = path {
                let p = self.source_map.map_target_to_client(&path);
                Some(json!({"path": p}))
            } else if let Some(source_reference) = source_reference {
                let addr = f.ip.as_usize();
                let name = self
                    .disasm_cache_by_addr
                    .get(&addr)
                    .map(|entry| entry.name.clone())
                    .unwrap_or_else(|| format!("disasm @ 0x{addr:x}"));
                Some(json!({"name": name, "sourceReference": source_reference}))
            } else {
                None
            };
            frames.push(json!({
                "id": frame_id,
                "name": name,
                "source": source,
                "line": line.unwrap_or(0),
                "column": col.unwrap_or(0),
            }));
        }
        self.send_success_body(
            req,
            json!({"stackFrames": frames, "totalFrames": frames.len()}),
        )
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
        let locals_ref = if let Some(r) = self
            .scope_cache
            .get(&(thread_id, frame_num, ScopeKind::Locals))
            .copied()
        {
            r
        } else {
            let locals = read_locals(dbg).unwrap_or_default();
            let r = self.vars.alloc(locals);
            self.scope_cache
                .insert((thread_id, frame_num, ScopeKind::Locals), r);
            r
        };

        let args_ref = if let Some(r) = self
            .scope_cache
            .get(&(thread_id, frame_num, ScopeKind::Arguments))
            .copied()
        {
            r
        } else {
            let args = read_args(dbg).unwrap_or_default();
            let r = self.vars.alloc(args);
            self.scope_cache
                .insert((thread_id, frame_num, ScopeKind::Arguments), r);
            r
        };

        let scopes = vec![
            json!({"name": "Locals", "variablesReference": locals_ref, "expensive": false}),
            json!({"name": "Arguments", "variablesReference": args_ref, "expensive": false}),
        ];

        self.send_success_body(req, json!({"scopes": scopes}))
    }

    fn handle_variables(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let variables_reference = req
            .arguments
            .get("variablesReference")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow!("variables: missing arguments.variablesReference"))?;

        let vars = self
            .vars
            .get(variables_reference)
            .cloned()
            .unwrap_or_default();

        let mut out = Vec::new();
        for (index, v) in vars.into_iter().enumerate() {
            let child_ref = if let Some(child) = v.child.as_ref() {
                if let Some(r) = self.child_links.get(&(variables_reference, index)).copied() {
                    r
                } else {
                    let r = self.vars.alloc(child.clone());
                    self.child_links.insert((variables_reference, index), r);
                    r
                }
            } else {
                0
            };
            out.push(json!({
                "name": v.name,
                "value": v.value,
                "type": v.type_name,
                "variablesReference": child_ref,
            }));
        }

        self.send_success_body(req, json!({"variables": out}))
    }

    fn handle_set_variable(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let vars_ref = req
            .arguments
            .get("variablesReference")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow!("setVariable: missing arguments.variablesReference"))?;

        let name = req
            .arguments
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("setVariable: missing arguments.name"))?
            .to_string();

        let new_value = req
            .arguments
            .get("value")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("setVariable: missing arguments.value"))?
            .to_string();

        let dbg = self
            .debugger
            .as_ref()
            .ok_or_else(|| anyhow!("setVariable: debugger not initialized"))?;

        let mut child_ref_to_remove = None;
        let (reply_value, reply_type) = {
            let vars = self
                .vars
                .get_mut(vars_ref)
                .ok_or_else(|| anyhow!("setVariable: unknown variablesReference={vars_ref}"))?;

            let (index, item) = vars
                .iter_mut()
                .enumerate()
                .find(|(_, v)| v.name == name)
                .ok_or_else(|| anyhow!("setVariable: variable '{name}' not found"))?;

            let Some(write) = item.write.clone() else {
                self.send_err(
                    req,
                    "setVariable: target variable is not writable".to_string(),
                )?;
                return Ok(());
            };

            match write {
                WriteMeta::Scalar { addr, kind } => {
                    let bytes = parse_set_value(kind, &new_value)?;
                    write_bytes(dbg, addr, &bytes)?;
                }
                WriteMeta::Composite { addr, type_graph } => {
                    let Some(source) = item.source.as_ref() else {
                        self.send_err(
                            req,
                            "setVariable: target variable is missing source value".to_string(),
                        )?;
                        return Ok(());
                    };
                    let Some(type_id) = source.type_id() else {
                        self.send_err(
                            req,
                            "setVariable: target variable has no type id".to_string(),
                        )?;
                        return Ok(());
                    };
                    let serialized = debugger::variable::value::serialize::serialize_dap_value(
                        &new_value,
                        &type_graph,
                        type_id,
                        Some(source),
                    )
                    .map_err(|err| anyhow!("setVariable: {err}"))?;
                    write_bytes(dbg, addr, &serialized.bytes)?;
                    item.child = None;
                    if let Some(child_ref) = self.child_links.remove(&(vars_ref, index)) {
                        child_ref_to_remove = Some(child_ref);
                    }
                }
            }

            // Update cached presentation value for this stop epoch.
            item.value = new_value.clone();
            (item.value.clone(), item.type_name.clone())
        };
        if let Some(child_ref) = child_ref_to_remove {
            self.vars.remove(child_ref);
        }

        self.send_success_body(
            req,
            json!({
                "value": reply_value,
                "type": reply_type,
                "variablesReference": 0,
            }),
        )
    }

    fn terminate_debuggee(&mut self) {
        // Drop the debugger instance. For internally spawned debuggee this will SIGKILL and detach ptrace in Debugger::drop.
        // For external debuggee it will detach.
        let _ = self.debugger.take();
        self.enqueue_event(InternalEvent::Terminated);
    }

    fn handle_terminate(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        self.send_success(req)?;
        self.terminate_debuggee();
        self.drain_events()?;
        Ok(())
    }

    fn handle_disconnect(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let terminate = req
            .arguments
            .get("terminateDebuggee")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        self.send_success(req)?;
        if terminate {
            self.terminate_debuggee();
            self.drain_events()?;
        } else if let Some(mut dbg) = self.debugger.take() {
            dbg.detach().context("detach debuggee")?;
        }
        Ok(())
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

    fn handle_source(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let args = req
            .arguments
            .as_object()
            .ok_or_else(|| anyhow!("source: arguments must be object"))?;

        let source_obj = args.get("source").and_then(serde_json::Value::as_object);

        let source_reference = args
            .get("sourceReference")
            .and_then(serde_json::Value::as_i64)
            .or_else(|| {
                source_obj
                    .and_then(|obj| obj.get("sourceReference"))
                    .and_then(serde_json::Value::as_i64)
            })
            .filter(|value| *value > 0);

        if let Some(source_reference) = source_reference {
            if let Some(disasm) = self.disasm_cache_by_reference.get(&source_reference) {
                return self.send_success_body(
                    req,
                    serde_json::json!({
                        "content": disasm.content,
                        "mimeType": "text/x-asm"
                    }),
                );
            }
            return self.send_success_body(
                req,
                serde_json::json!({
                    "content": format!(
                        "No cached disassembly found for sourceReference {source_reference}."
                    ),
                    "mimeType": "text/plain"
                }),
            );
        }

        let source_obj = source_obj.ok_or_else(|| anyhow!("source: missing source object"))?;
        let path = source_obj
            .get("path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow!("source: missing source.path"))?;

        let read_source = |candidate: &str| -> Option<String> {
            let normalized = SourceMap::norm_path(candidate);
            // VSCode sometimes sends relative glibc paths like "./nptl/pthread_kill.c"
            let try_paths = [
                std::path::PathBuf::from(&normalized),
                std::path::PathBuf::from(normalized.trim_start_matches("./")),
            ];

            for p in &try_paths {
                if let Ok(data) = std::fs::read_to_string(p) {
                    return Some(data);
                }
            }

            None
        };

        let mapped_path = self.source_map.map_client_to_target(path);
        let mut content = read_source(&mapped_path);
        if content.is_none() {
            let fallback_path = self.source_map.map_target_to_client(path);
            content = read_source(&fallback_path);
        }

        let Some(content) = content else {
            return self.send_err(
                req,
                format!(
                    "Could not load source '{}': file not found on adapter host",
                    path
                ),
            );
        };

        self.send_success_body(
            req,
            serde_json::json!({
                "content": content,
                "mimeType": "text/x-c"
            }),
        )
    }

    fn handle_disassemble(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let dbg = self
            .debugger
            .as_ref()
            .ok_or_else(|| anyhow!("disassemble: debugger not initialized"))?;
        let args = req
            .arguments
            .as_object()
            .ok_or_else(|| anyhow!("disassemble: arguments must be object"))?;

        let memory_reference = args
            .get("memoryReference")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow!("disassemble: missing arguments.memoryReference"))?;

        let instruction_count = args
            .get("instructionCount")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| anyhow!("disassemble: missing arguments.instructionCount"))?;
        if instruction_count <= 0 {
            return self.send_err(
                req,
                "disassemble: instructionCount must be positive".to_string(),
            );
        }

        let offset = args
            .get("offset")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);

        let instruction_offset = args
            .get("instructionOffset")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);

        let base_addr = parse_memory_reference(memory_reference)
            .context("disassemble: invalid memoryReference")?;
        let start = offset
            .checked_add(base_addr as i64)
            .ok_or_else(|| anyhow!("disassemble: address overflow"))?;
        if start < 0 {
            return self.send_err(req, "disassemble: start address is negative".to_string());
        }
        let anchor_addr = start as usize;
        let back_instructions = instruction_offset.unsigned_abs() as usize;
        let max_len = 16usize;
        let start_addr = anchor_addr.saturating_sub(back_instructions.saturating_mul(max_len));
        let disasm_count = instruction_count as usize + back_instructions + 16;
        let instructions = disassemble_from_address(dbg, start_addr, disasm_count)?;
        let anchor_index = instructions
            .iter()
            .position(|ins| ins.address as usize >= anchor_addr)
            .unwrap_or(instructions.len());
        let start_index = if instruction_offset >= 0 {
            anchor_index.saturating_add(instruction_offset as usize)
        } else {
            anchor_index.saturating_sub(back_instructions)
        };

        let instructions = instructions
            .into_iter()
            .skip(start_index)
            .take(instruction_count as usize)
            .map(|ins| {
                json!({
                    "address": format!("0x{:x}", ins.address),
                    "instructionBytes": ins.bytes_hex,
                    "instruction": ins.text,
                })
            })
            .collect::<Vec<_>>();

        self.send_success_body(req, json!({ "instructions": instructions }))
    }

    fn disasm_source_for_address(&mut self, addr: usize) -> anyhow::Result<DisasmSource> {
        if let Some(existing) = self.disasm_cache_by_addr.get(&addr) {
            return Ok(existing.clone());
        }

        let dbg = self
            .debugger
            .as_ref()
            .ok_or_else(|| anyhow!("disassemble: debugger not initialized"))?;
        let instructions = disassemble_from_address(dbg, addr, 64)?;
        let content = if instructions.is_empty() {
            format!("No disassembly available at 0x{addr:x}.")
        } else {
            instructions
                .iter()
                .map(|ins| format!("0x{:x}: {}", ins.address, ins.text))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let reference = self.next_source_reference;
        self.next_source_reference += 1;
        let name = format!("disasm @ 0x{addr:x}");
        let entry = DisasmSource {
            reference,
            name,
            content,
        };
        self.disasm_cache_by_addr.insert(addr, entry.clone());
        self.disasm_cache_by_reference
            .insert(reference, entry.clone());
        Ok(entry)
    }
}

struct DisasmInstruction {
    address: u64,
    bytes_hex: String,
    text: String,
}

fn completion_prefix(text: &str, column: Option<i64>) -> (String, i64, i64) {
    let chars: Vec<char> = text.chars().collect();
    let max_col = chars.len() as i64 + 1;
    let column = column.unwrap_or(max_col).clamp(1, max_col);
    let end_idx = (column - 1) as usize;
    let mut start_idx = end_idx;
    while start_idx > 0 {
        let c = chars[start_idx - 1];
        if c.is_ascii_alphanumeric() || c == '_' || c == ':' {
            start_idx -= 1;
        } else {
            break;
        }
    }
    let prefix: String = chars[start_idx..end_idx].iter().collect();
    let length = (end_idx - start_idx) as i64;
    let start_column = start_idx as i64 + 1;
    (prefix, start_column, length)
}

fn parse_call_target_addr(operands: &str) -> Option<u64> {
    let idx = operands.find("0x")?;
    let hex = operands[idx + 2..]
        .chars()
        .take_while(|c| c.is_ascii_hexdigit())
        .collect::<String>();
    if hex.is_empty() {
        return None;
    }
    u64::from_str_radix(&hex, 16).ok()
}

fn disassemble_from_address(
    dbg: &debugger::Debugger,
    addr: usize,
    instruction_count: usize,
) -> anyhow::Result<Vec<DisasmInstruction>> {
    let cs = Capstone::new()
        .x86()
        .mode(arch::x86::ArchMode::Mode64)
        .syntax(arch::x86::ArchSyntax::Att)
        .build()
        .map_err(|err| anyhow!("disassemble: init capstone: {err}"))?;
    let max_len = 16usize;
    let read_len = instruction_count.saturating_mul(max_len).max(max_len);
    let bytes = dbg
        .read_memory(addr, read_len)
        .context("disassemble: read_memory")?;
    let insns = cs
        .disasm_all(&bytes, addr as u64)
        .map_err(|err| anyhow!("disassemble: disasm_all: {err}"))?;

    let mut out = Vec::new();
    for insn in insns.iter().take(instruction_count) {
        let mnemonic: &str = insn.mnemonic().unwrap_or("<unknown>");
        let op_str = insn.op_str().unwrap_or("");
        let text = if op_str.is_empty() {
            mnemonic.to_string()
        } else {
            format!("{mnemonic} {op_str}")
        };
        let bytes_hex = insn
            .bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join("");
        out.push(DisasmInstruction {
            address: insn.address(),
            bytes_hex,
            text,
        });
    }
    Ok(out)
}

fn disassemble_from_range(
    dbg: &debugger::Debugger,
    start_addr: usize,
    end_addr: usize,
) -> anyhow::Result<Vec<DisasmInstruction>> {
    if end_addr <= start_addr {
        return Ok(Vec::new());
    }
    let len = end_addr - start_addr;
    let max_len = 0x10000usize;
    let read_len = len.min(max_len);
    let cs = Capstone::new()
        .x86()
        .mode(arch::x86::ArchMode::Mode64)
        .syntax(arch::x86::ArchSyntax::Att)
        .build()
        .map_err(|err| anyhow!("disassemble: init capstone: {err}"))?;
    let bytes = dbg
        .read_memory(start_addr, read_len)
        .context("disassemble: read_memory")?;
    let insns = cs
        .disasm_all(&bytes, start_addr as u64)
        .map_err(|err| anyhow!("disassemble: disasm_all: {err}"))?;

    let mut out = Vec::new();
    for insn in insns.iter() {
        let mnemonic: &str = insn.mnemonic().unwrap_or("<unknown>");
        let op_str = insn.op_str().unwrap_or("");
        let text = if op_str.is_empty() {
            mnemonic.to_string()
        } else {
            format!("{mnemonic} {op_str}")
        };
        let bytes_hex = insn
            .bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join("");
        out.push(DisasmInstruction {
            address: insn.address(),
            bytes_hex,
            text,
        });
    }
    Ok(out)
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

fn render_value_to_string(v: &debugger::variable::value::Value) -> String {
    use debugger::variable::render::RenderValue;
    match v.value_layout() {
        Some(debugger::variable::render::ValueLayout::PreRendered(s)) => s.to_string(),
        Some(debugger::variable::render::ValueLayout::Referential(ptr)) => {
            format!("{ptr:p}")
        }
        Some(debugger::variable::render::ValueLayout::Wrapped(inner)) => {
            render_value_to_string(inner)
        }
        Some(debugger::variable::render::ValueLayout::Structure(_)) => "{...}".to_string(),
        Some(debugger::variable::render::ValueLayout::IndexedList(_)) => "[...]".to_string(),
        Some(debugger::variable::render::ValueLayout::NonIndexedList(_)) => "[...]".to_string(),
        Some(debugger::variable::render::ValueLayout::Map(_)) => "{...}".to_string(),
        None => "<unavailable>".to_string(),
    }
}

fn value_write_meta(
    v: &debugger::variable::value::Value,
    type_graph: Rc<debugger::ComplexType>,
) -> Option<WriteMeta> {
    use debugger::variable::value::{SupportedScalar, Value as BsValue};

    let addr = v.in_memory_location()?;
    match v {
        BsValue::Scalar(s) => match s.value.as_ref()? {
            SupportedScalar::I8(_) => Some(WriteMeta::Scalar {
                addr,
                kind: ScalarKind::I8,
            }),
            SupportedScalar::I16(_) => Some(WriteMeta::Scalar {
                addr,
                kind: ScalarKind::I16,
            }),
            SupportedScalar::I32(_) => Some(WriteMeta::Scalar {
                addr,
                kind: ScalarKind::I32,
            }),
            SupportedScalar::I64(_) => Some(WriteMeta::Scalar {
                addr,
                kind: ScalarKind::I64,
            }),
            SupportedScalar::I128(_) => Some(WriteMeta::Scalar {
                addr,
                kind: ScalarKind::I128,
            }),
            SupportedScalar::Isize(_) => Some(WriteMeta::Scalar {
                addr,
                kind: ScalarKind::Isize,
            }),
            SupportedScalar::U8(_) => Some(WriteMeta::Scalar {
                addr,
                kind: ScalarKind::U8,
            }),
            SupportedScalar::U16(_) => Some(WriteMeta::Scalar {
                addr,
                kind: ScalarKind::U16,
            }),
            SupportedScalar::U32(_) => Some(WriteMeta::Scalar {
                addr,
                kind: ScalarKind::U32,
            }),
            SupportedScalar::U64(_) => Some(WriteMeta::Scalar {
                addr,
                kind: ScalarKind::U64,
            }),
            SupportedScalar::U128(_) => Some(WriteMeta::Scalar {
                addr,
                kind: ScalarKind::U128,
            }),
            SupportedScalar::Usize(_) => Some(WriteMeta::Scalar {
                addr,
                kind: ScalarKind::Usize,
            }),
            SupportedScalar::F32(_) => Some(WriteMeta::Scalar {
                addr,
                kind: ScalarKind::F32,
            }),
            SupportedScalar::F64(_) => Some(WriteMeta::Scalar {
                addr,
                kind: ScalarKind::F64,
            }),
            SupportedScalar::Bool(_) => Some(WriteMeta::Scalar {
                addr,
                kind: ScalarKind::Bool,
            }),
            SupportedScalar::Char(_) => Some(WriteMeta::Scalar {
                addr,
                kind: ScalarKind::Char,
            }),
            SupportedScalar::Empty() => None,
        },
        _ => v
            .type_id()
            .map(|_| WriteMeta::Composite { addr, type_graph }),
    }
}

fn parse_set_value(kind: ScalarKind, input: &str) -> anyhow::Result<Vec<u8>> {
    let s = input.trim();

    fn parse_int_i128(s: &str) -> anyhow::Result<i128> {
        if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            i128::from_str_radix(hex, 16).context("hex i128 parse")
        } else {
            s.parse::<i128>().context("dec i128 parse")
        }
    }

    fn parse_int_u128(s: &str) -> anyhow::Result<u128> {
        if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            u128::from_str_radix(hex, 16).context("hex u128 parse")
        } else {
            s.parse::<u128>().context("dec u128 parse")
        }
    }

    match kind {
        ScalarKind::I8 => Ok(vec![(parse_int_i128(s)? as i8) as u8]),
        ScalarKind::U8 => Ok(vec![parse_int_u128(s)? as u8]),
        ScalarKind::I16 => Ok((parse_int_i128(s)? as i16).to_le_bytes().to_vec()),
        ScalarKind::U16 => Ok((parse_int_u128(s)? as u16).to_le_bytes().to_vec()),
        ScalarKind::I32 => Ok((parse_int_i128(s)? as i32).to_le_bytes().to_vec()),
        ScalarKind::U32 => Ok((parse_int_u128(s)? as u32).to_le_bytes().to_vec()),
        ScalarKind::I64 => Ok((parse_int_i128(s)? as i64).to_le_bytes().to_vec()),
        ScalarKind::U64 => Ok((parse_int_u128(s)? as u64).to_le_bytes().to_vec()),
        ScalarKind::I128 => Ok(parse_int_i128(s)?.to_le_bytes().to_vec()),
        ScalarKind::U128 => Ok(parse_int_u128(s)?.to_le_bytes().to_vec()),
        ScalarKind::Isize => Ok((parse_int_i128(s)? as isize).to_le_bytes().to_vec()),
        ScalarKind::Usize => Ok((parse_int_u128(s)? as usize).to_le_bytes().to_vec()),
        ScalarKind::F32 => Ok(s
            .parse::<f32>()
            .context("f32 parse")?
            .to_le_bytes()
            .to_vec()),
        ScalarKind::F64 => Ok(s
            .parse::<f64>()
            .context("f64 parse")?
            .to_le_bytes()
            .to_vec()),
        ScalarKind::Bool => {
            let b = match s {
                "true" | "True" | "TRUE" => true,
                "false" | "False" | "FALSE" => false,
                "1" => true,
                "0" => false,
                _ => anyhow::bail!("bool parse: expected true/false/0/1, got '{s}'"),
            };
            Ok(vec![if b { 1 } else { 0 }])
        }
        ScalarKind::Char => {
            // Accept: 'a' or 97 or 0x61. Stored as Rust char (u32).
            if let Some(stripped) = s.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')) {
                let mut it = stripped.chars();
                let ch = it.next().context("char parse: empty literal")?;
                if it.next().is_some() {
                    anyhow::bail!("char parse: expected single char literal");
                }
                let u = ch as u32;
                Ok(u.to_le_bytes().to_vec())
            } else if s.chars().count() == 1 {
                let u = s.chars().next().unwrap() as u32;
                Ok(u.to_le_bytes().to_vec())
            } else {
                let u = parse_int_u128(s)? as u32;
                Ok(u.to_le_bytes().to_vec())
            }
        }
    }
}

fn write_bytes(dbg: &debugger::Debugger, addr: usize, bytes: &[u8]) -> anyhow::Result<()> {
    let word = std::mem::size_of::<usize>();
    if bytes.is_empty() {
        return Ok(());
    }

    let start = addr;
    let end = addr + bytes.len();
    let mut cur = start;

    while cur < end {
        let word_start = (cur / word) * word;
        let word_end = word_start + word;
        let chunk_from = std::cmp::max(cur, word_start);
        let chunk_to = std::cmp::min(end, word_end);

        let mut existing = dbg.read_memory(word_start, word).context("read_memory")?;
        let src_off = chunk_from - start;
        let dst_off = chunk_from - word_start;
        existing[dst_off..dst_off + (chunk_to - chunk_from)]
            .copy_from_slice(&bytes[src_off..src_off + (chunk_to - chunk_from)]);

        let mut le = [0u8; std::mem::size_of::<usize>()];
        le.copy_from_slice(&existing[..word]);
        let value = usize::from_le_bytes(le);

        dbg.write_memory(word_start as _, value as _)
            .context("write_memory")?;

        cur = word_end;
    }

    Ok(())
}

fn value_children(
    v: &debugger::variable::value::Value,
    type_graph: Rc<debugger::ComplexType>,
) -> Option<Vec<VarItem>> {
    use debugger::variable::render::{RenderValue, ValueLayout};
    let layout = v.value_layout()?;
    match layout {
        ValueLayout::Structure(members) => {
            let mut out = Vec::new();
            for m in members {
                let field_name = m.field_name.as_deref().unwrap_or("<unnamed>").to_string();
                let val = &m.value;
                out.push(VarItem {
                    name: field_name,
                    value: render_value_to_string(val),
                    type_name: Some(val.r#type().to_string()),
                    child: value_children(val, type_graph.clone()),
                    write: value_write_meta(val, type_graph.clone()),
                    source: Some(val.clone()),
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
                    child: value_children(val, type_graph.clone()),
                    write: value_write_meta(val, type_graph.clone()),
                    source: Some(val.clone()),
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
                    child: value_children(val, type_graph.clone()),
                    write: value_write_meta(val, type_graph.clone()),
                    source: Some(val.clone()),
                });
            }
            Some(out)
        }
        ValueLayout::Map(kvs) => {
            let mut out = Vec::new();
            for (i, (k, val)) in kvs.iter().enumerate() {
                out.push(VarItem {
                    name: format!("[{i}]"),
                    value: format!(
                        "{} => {}",
                        render_value_to_string(k),
                        render_value_to_string(val)
                    ),
                    type_name: None,
                    child: None,
                    write: None,
                    source: None,
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
        let type_graph = Rc::new(r.type_graph().clone());
        let (id, val) = r.into_identified_value();
        let name = id.to_string();
        out.push(VarItem {
            name,
            value: render_value_to_string(&val),
            type_name: Some(val.r#type().to_string()),
            child: value_children(&val, type_graph.clone()),
            write: value_write_meta(&val, type_graph.clone()),
            source: Some(val.clone()),
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
        let type_graph = Rc::new(r.type_graph().clone());
        let (id, val) = r.into_identified_value();
        let name = id.to_string();
        out.push(VarItem {
            name,
            value: render_value_to_string(&val),
            type_name: Some(val.r#type().to_string()),
            child: value_children(&val, type_graph.clone()),
            write: value_write_meta(&val, type_graph.clone()),
            source: Some(val.clone()),
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

    let args = Args::parse();
    // Ensure Rust environment is initialised for non-CLI frontend.
    // This avoids panics in src/debugger/rust/mod.rs when core tries to access it.
    bugstalker::debugger::rust::Environment::init(None);
    let addr: SocketAddr = args.listen.parse().context("Invalid listen address")?;
    let listener = TcpListener::bind(addr).with_context(|| format!("bind {addr}"))?;
    info!(target: "dap", "bs-dap listening on {addr}");

    let tracer = match &args.log_file {
        Some(path) => Some(FileTracer::new(path)?),
        None => None,
    };
    if args.trace_dap && tracer.is_none() {
        warn!(target: "dap", "--trace-dap requires --log-file; tracing disabled");
    }

    // Server mode: accept multiple clients sequentially. One client == one debug session.
    loop {
        let (stream, peer) = match listener.accept() {
            Ok(v) => v,
            Err(err) => {
                warn!(target: "dap", "accept failed: {err:#}");
                continue;
            }
        };
        info!(target: "dap", "DAP client connected: {peer}");
        if let Some(t) = &tracer {
            t.line(&format!("client connected: {peer}"));
        }

        let io = match DapIo::new(stream, tracer.clone(), args.trace_dap) {
            Ok(v) => v,
            Err(err) => {
                warn!(target: "dap", "failed to init DAP I/O: {err:#}");
                continue;
            }
        };

        let res = DebugSession::new(io).run(args.oracle.clone());
        if let Err(err) = res {
            warn!(target: "dap", "session ended with error: {err:#}");
            if let Some(t) = &tracer {
                t.line(&format!("session error: {err:#}"));
            }
        } else if let Some(t) = &tracer {
            t.line("session finished OK");
        }

        if args.oneshot {
            break;
        }
    }
    Ok(())
}
