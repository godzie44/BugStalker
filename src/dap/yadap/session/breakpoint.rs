use crate::dap::yadap::protocol::{DapRequest, InternalEvent};
use crate::dap::yadap::session::DebugSession;
use crate::debugger;
use crate::debugger::address::RelocatedAddress;
use crate::debugger::register::debug::{BreakCondition, BreakSize};
use crate::debugger::variable::dqe::Dqe;
use crate::ui::command::parser::expression as bs_expr;
use crate::ui::command::parser::watchpoint_at_address;
use crate::ui::command::watch::WatchpointIdentity;
use anyhow::anyhow;
use chumsky::Parser as _;
use chumsky::prelude::end;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct BreakpointRecord {
    pub id: i64,
    pub addresses: Vec<debugger::address::Address>,
    pub condition: Option<String>,
    pub hit_condition: Option<HitCondition>,
    pub log_message: Option<String>,
    pub hit_count: u64,
}

#[derive(Debug, Clone)]
pub enum HitCondition {
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

    pub fn matches(&self, hits: u64) -> bool {
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
pub struct BreakpointOptions {
    pub condition: Option<String>,
    pub hit_condition: Option<HitCondition>,
    pub log_message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BreakpointHitInfo {
    pub id: i64,
    pub condition: Option<String>,
    pub hit_condition: Option<HitCondition>,
    pub log_message: Option<String>,
    pub hit_count: u64,
}

#[derive(Clone)]
pub enum DataBreakpointTarget {
    Expr { expression: String, dqe: Dqe },
    Address { addr: usize, size: u8 },
}

#[derive(Clone)]
pub struct DataBreakpointRecord {
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

impl DebugSession {
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

    pub(super) fn handle_set_breakpoints(&mut self, req: &DapRequest) -> anyhow::Result<()> {
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

    pub(super) fn handle_set_function_breakpoints(
        &mut self,
        req: &DapRequest,
    ) -> anyhow::Result<()> {
        let prev = std::mem::take(&mut self.function_breakpoints);
        let bps = req
            .arguments
            .get("breakpoints")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let bps_len = bps.len();

        let progress_id = if bps.is_empty() {
            None
        } else {
            Some(self.enqueue_progress_start(
                "Searching symbols",
                Some("Resolving function breakpoints".to_string()),
                Some(0),
            ))
        };
        if progress_id.is_some() {
            self.drain_events()?;
        }

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

        if let Some(progress_id) = progress_id {
            self.enqueue_progress_update(
                progress_id.clone(),
                Some(format!("Resolved {} breakpoint(s)", bps_len)),
                Some(100),
            );
            self.enqueue_progress_end(
                progress_id,
                Some("Function breakpoint resolution complete".to_string()),
            );
            self.drain_events()?;
        }

        self.send_success_body(req, json!({"breakpoints": rsp_bps}))?;
        self.drain_events()
    }

    pub(super) fn handle_set_instruction_breakpoints(
        &mut self,
        req: &DapRequest,
    ) -> anyhow::Result<()> {
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
                let addr = match super::parse_memory_reference_with_offset(reference, offset) {
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

    pub(super) fn handle_set_exception_breakpoints(
        &mut self,
        req: &DapRequest,
    ) -> anyhow::Result<()> {
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

    pub(super) fn handle_data_breakpoint_info(&mut self, req: &DapRequest) -> anyhow::Result<()> {
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

    pub(super) fn handle_set_data_breakpoints(&mut self, req: &DapRequest) -> anyhow::Result<()> {
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

    pub(super) fn handle_breakpoint_locations(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let args = req
            .arguments
            .as_object()
            .ok_or_else(|| anyhow!("breakpointLocations: arguments must be object"))?;

        if let Some(source) = args.get("source") {
            let dbg = self
                .debugger
                .as_ref()
                .ok_or_else(|| anyhow!("breakpointLocations: debugger not initialized"))?;
            let source_path = source
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("breakpointLocations: missing source.path"))?;
            if source_path.is_empty() {
                return self.send_err(req, "breakpointLocations: source.path must not be empty");
            }
            let line = args
                .get("line")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| anyhow!("breakpointLocations: missing line"))?;
            if line < 1 {
                return self.send_err(req, "breakpointLocations: line must be >= 1");
            }
            if args.get("endLine").is_some()
                && args.get("endLine").and_then(|v| v.as_i64()).is_none()
            {
                return self.send_err(req, "breakpointLocations: endLine must be an integer");
            }
            let end_line = args.get("endLine").and_then(|v| v.as_i64()).unwrap_or(line);
            if end_line < line {
                return self.send_err(req, "breakpointLocations: endLine must be >= line");
            }

            if args.get("column").is_some() && args.get("column").and_then(|v| v.as_i64()).is_none()
            {
                return self.send_err(req, "breakpointLocations: column must be an integer");
            }
            let column = args.get("column").and_then(|v| v.as_i64());
            if let Some(column) = column
                && column < 1
            {
                return self.send_err(req, "breakpointLocations: column must be >= 1");
            }
            if args.get("endColumn").is_some()
                && args.get("endColumn").and_then(|v| v.as_i64()).is_none()
            {
                return self.send_err(req, "breakpointLocations: endColumn must be an integer");
            }
            let end_column = args.get("endColumn").and_then(|v| v.as_i64()).or(column);
            if let (Some(column), Some(end_column)) = (column, end_column)
                && end_column < column
            {
                return self.send_err(req, "breakpointLocations: endColumn must be >= column");
            }

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
            if reference.is_empty() {
                return self.send_err(
                    req,
                    "breakpointLocations: instructionReference must not be empty",
                );
            }
            if args.get("offset").is_some() && args.get("offset").and_then(|v| v.as_i64()).is_none()
            {
                return self.send_err(req, "breakpointLocations: offset must be an integer");
            }
            if args.get("endOffset").is_some()
                && args.get("endOffset").and_then(|v| v.as_i64()).is_none()
            {
                return self.send_err(req, "breakpointLocations: endOffset must be an integer");
            }
            let offset = args.get("offset").and_then(|v| v.as_i64()).unwrap_or(0);
            let end_offset = args.get("endOffset").and_then(|v| v.as_i64());
            let start_addr = super::parse_memory_reference_with_offset(reference, offset)?;

            let progress_id = self.enqueue_progress_start(
                "Disassembling",
                Some("Resolving breakpoint locations".to_string()),
                Some(0),
            );
            self.drain_events()?;

            let mut breakpoints = Vec::new();
            let mut seen = HashSet::new();
            if let Some(end_offset) = end_offset {
                if end_offset < offset {
                    return self.send_err(req, "breakpointLocations: endOffset is before offset");
                }
                let end_addr = super::parse_memory_reference_with_offset(reference, end_offset)?;
                let end_exclusive = end_addr.saturating_add(1);
                let dbg = self
                    .debugger
                    .as_ref()
                    .ok_or_else(|| anyhow!("breakpointLocations: debugger not initialized"))?;
                let instructions = super::source::disassemble_from_range(
                    dbg,
                    start_addr,
                    end_exclusive,
                    super::MEMORY_READ_TIMEOUT,
                )?;
                for ins in instructions {
                    if !seen.insert(ins.address) {
                        continue;
                    }
                    breakpoints.push(json!({
                        "instructionReference": format!("0x{:x}", ins.address),
                    }));
                }
            } else {
                let dbg = self
                    .debugger
                    .as_ref()
                    .ok_or_else(|| anyhow!("breakpointLocations: debugger not initialized"))?;
                let instructions = super::source::disassemble_from_address(
                    dbg,
                    start_addr,
                    1,
                    super::MEMORY_READ_TIMEOUT,
                )?;
                for ins in instructions {
                    if !seen.insert(ins.address) {
                        continue;
                    }
                    breakpoints.push(json!({
                        "instructionReference": format!("0x{:x}", ins.address),
                    }));
                }
            }

            self.enqueue_progress_update(
                progress_id.clone(),
                Some(format!("Found {} location(s)", breakpoints.len())),
                Some(100),
            );
            self.enqueue_progress_end(progress_id, Some("Disassembly complete".to_string()));
            self.drain_events()?;
            return self.send_success_body(req, json!({ "breakpoints": breakpoints }));
        }

        self.send_err(
            req,
            "breakpointLocations: missing source or instructionReference",
        )
    }
}
