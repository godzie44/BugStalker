use super::ThreadFocusByPid;
use crate::dap::yadap::protocol::DapRequest;
use crate::debugger;
use crate::debugger::variable::render::RenderValue;
use crate::ui::command::parser::expression as bs_expr;
use anyhow::{Context, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_ENGINE};
use chumsky::Parser as _;
use nix::unistd::Pid;
use serde_json::json;
use std::rc::Rc;
use std::time::Instant;

#[derive(Clone)]
pub enum WriteMeta {
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
pub enum ScalarKind {
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
pub struct VarItem {
    pub name: String,
    pub value: String,
    pub type_name: Option<String>,
    pub child: Option<Vec<VarItem>>,
    pub write: Option<WriteMeta>,
    pub source: Option<debugger::variable::value::Value>,
}

impl super::DebugSession {
    pub(super) fn handle_variables(&mut self, req: &DapRequest) -> anyhow::Result<()> {
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

    pub(super) fn handle_set_variable(&mut self, req: &DapRequest) -> anyhow::Result<()> {
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
        )?;
        self.enqueue_invalidated(vec![
            "variables".to_string(),
            "stack".to_string(),
            "memory".to_string(),
        ]);
        self.drain_events()
    }

    pub(super) fn handle_read_memory(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        if self.consume_cancellation(req, None)? {
            return Ok(());
        }
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

        let addr = super::parse_memory_reference_with_offset(memory_reference, offset)
            .context("readMemory: invalid memoryReference")?;
        let start = Instant::now();
        let bytes = dbg
            .read_memory(addr, count as usize)
            .context("readMemory: read_memory")?;
        let elapsed = start.elapsed();
        if elapsed > super::MEMORY_READ_TIMEOUT {
            return self.send_err(
                req,
                format!(
                    "readMemory: read timed out after {}ms",
                    super::MEMORY_READ_TIMEOUT.as_millis()
                ),
            );
        }
        if self.consume_cancellation(req, None)? {
            return Ok(());
        }
        let data = BASE64_ENGINE.encode(bytes);
        self.send_success_body(
            req,
            json!({
                "address": format!("0x{addr:x}"),
                "data": data,
            }),
        )
    }

    pub(super) fn handle_write_memory(&mut self, req: &DapRequest) -> anyhow::Result<()> {
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

        let addr = super::parse_memory_reference_with_offset(memory_reference, offset)
            .context("writeMemory: invalid memoryReference")?;
        let bytes = BASE64_ENGINE
            .decode(data)
            .map_err(|err| anyhow!("writeMemory: base64 decode failed: {err}"))?;
        write_bytes(dbg, addr, &bytes).context("writeMemory: write_bytes")?;
        self.send_success_body(req, json!({ "bytesWritten": bytes.len() }))?;
        self.enqueue_invalidated(vec!["memory".to_string()]);
        self.drain_events()
    }

    pub(super) fn handle_set_expression(&mut self, req: &DapRequest) -> anyhow::Result<()> {
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

        if let Some(frame_value) = req.arguments.get("frameId") {
            let frame_id = frame_value
                .as_i64()
                .ok_or_else(|| anyhow!("setExpression: frameId must be an integer"))?;
            if frame_id < 0 {
                return self.send_err(req, "setExpression: frameId must be non-negative");
            }
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

        self.send_success_body(req, response)?;
        self.enqueue_invalidated(vec![
            "variables".to_string(),
            "stack".to_string(),
            "memory".to_string(),
        ]);
        self.drain_events()
    }

    pub(super) fn handle_evaluate(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        if self.consume_cancellation(req, None)? {
            return Ok(());
        }
        let expression = req
            .arguments
            .get("expression")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("evaluate: missing arguments.expression"))?;

        let (body, elapsed) = {
            let dbg = self
                .debugger
                .as_mut()
                .ok_or_else(|| anyhow!("evaluate: debugger not initialized"))?;

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

            let start = Instant::now();
            let results = dbg.read_variable(dqe).context("evaluate read_variable")?;
            let elapsed = start.elapsed();
            let body = if results.is_empty() {
                json!({"result": "<no result>", "variablesReference": 0})
            } else {
                let result = results.into_iter().next().unwrap();
                let type_graph = Rc::new(result.type_graph().clone());
                let (_id, val) = result.into_identified_value();
                let child = value_children(&val, type_graph);
                let vars_ref = child.map(|c| self.vars.alloc(c)).unwrap_or(0);
                let result_str = render_value_to_string(&val);
                json!({"result": result_str, "variablesReference": vars_ref})
            };
            (body, elapsed)
        };
        if elapsed > super::DEBUGGER_RESPONSE_TIMEOUT {
            return self.send_err(
                req,
                format!(
                    "evaluate: debugger response timed out after {}ms",
                    super::DEBUGGER_RESPONSE_TIMEOUT.as_millis()
                ),
            );
        }
        if self.consume_cancellation(req, None)? {
            return Ok(());
        }
        self.send_success_body(req, body)
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

pub fn render_value_to_string(v: &debugger::variable::value::Value) -> String {
    use debugger::variable::render::RenderValue;
    match v.value_layout() {
        Some(debugger::variable::render::ValueLayout::PreRendered(s)) => s.to_string(),
        Some(debugger::variable::render::ValueLayout::Referential(ptr)) => {
            format!("{ptr:p}")
        }
        Some(debugger::variable::render::ValueLayout::Wrapped(inner)) => {
            format!(
                "{}::{}",
                RenderValue::r#type(inner).name_fmt(),
            render_value_to_string(inner)
            )
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
        ValueLayout::Wrapped(v) => value_children(v, type_graph),
        _ => None,
    }
}

pub fn read_locals(dbg: &debugger::Debugger) -> anyhow::Result<Vec<VarItem>> {
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

pub fn read_args(dbg: &debugger::Debugger) -> anyhow::Result<Vec<VarItem>> {
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
