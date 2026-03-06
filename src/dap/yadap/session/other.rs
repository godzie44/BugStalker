use crate::dap::yadap::protocol::DapRequest;
use crate::dap::yadap::session::ThreadFocusByPid;
use anyhow::{Context, anyhow};
use nix::unistd::Pid;
use regex::escape as regex_escape;
use serde_json::json;
use std::collections::HashSet;
use std::process::{Command, Stdio};

impl super::DebugSession {
    pub(super) fn handle_cancel(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        if req.arguments.is_null() {
            return self.send_success(req);
        }
        let args = req
            .arguments
            .as_object()
            .ok_or_else(|| anyhow!("cancel: arguments must be object"))?;
        if let Some(request_id) = args.get("requestId") {
            let request_id = request_id
                .as_i64()
                .ok_or_else(|| anyhow!("cancel: requestId must be an integer"))?;
            self.canceled_request_ids.insert(request_id);
        }
        if let Some(progress_id) = args.get("progressId") {
            let progress_id = progress_id
                .as_str()
                .ok_or_else(|| anyhow!("cancel: progressId must be a string"))?;
            self.canceled_progress_ids.insert(progress_id.to_string());
        }

        self.send_success(req)
    }

    pub(super) fn handle_run_in_terminal(&mut self, req: &DapRequest) -> anyhow::Result<()> {
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
        if let Some(kind) = args.get("kind")
            && kind.as_str().is_none()
        {
            return self.send_err(req, "runInTerminal: kind must be a string");
        }
        if let Some(title) = args.get("title")
            && title.as_str().is_none()
        {
            return self.send_err(req, "runInTerminal: title must be a string");
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
        } else if args.get("cwd").is_some() {
            return self.send_err(req, "runInTerminal: cwd must be a string");
        }
        if let Some(env) = args.get("env").and_then(|v| v.as_object()) {
            for (key, value) in env {
                let Some(value) = value.as_str() else {
                    return self.send_err(req, "runInTerminal: env values must be strings");
                };
                command.env(key, value);
            }
        } else if args.get("env").is_some() {
            return self.send_err(req, "runInTerminal: env must be an object");
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

    pub(super) fn handle_exception_info(&mut self, req: &DapRequest) -> anyhow::Result<()> {
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

    pub(super) fn handle_completions(&mut self, req: &DapRequest) -> anyhow::Result<()> {
        let args = req
            .arguments
            .as_object()
            .ok_or_else(|| anyhow!("completions: arguments must be object (possibly empty)"))?;
        let text = args
            .get("text")
            .ok_or_else(|| anyhow!("completions: missing arguments.text"))?
            .as_str()
            .ok_or_else(|| anyhow!("completions: text must be a string"))?;
        let column = args
            .get("column")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow!("completions: missing arguments.column"))?;
        if column < 1 {
            return self.send_err(req, "completions: column must be >= 1");
        }
        let column = Some(column);
        if let Some(frame_value) = args.get("frameId") {
            let frame_id = frame_value
                .as_i64()
                .ok_or_else(|| anyhow!("completions: frameId must be an integer"))?;
            if frame_id < 0 {
                return self.send_err(req, "completions: frameId must be non-negative");
            }
        }
        let (prefix, start_column, prefix_len) = completion_prefix(text, column);

        let mut targets = Vec::new();
        let mut seen = HashSet::new();

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

        {
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

            if let Ok(locals) = super::data::read_locals(dbg) {
                for v in locals {
                    if prefix.is_empty() || v.name.starts_with(&prefix) {
                        push_item(v.name);
                    }
                }
            }

            if let Ok(args_vars) = super::data::read_args(dbg) {
                for v in args_vars {
                    if prefix.is_empty() || v.name.starts_with(&prefix) {
                        push_item(v.name);
                    }
                }
            }
        }

        if !prefix.is_empty() {
            let regex = format!("^{}", regex_escape(&prefix));
            let progress_id = self.enqueue_progress_start(
                "Indexing symbols",
                Some(format!("Searching for '{prefix}'")),
                Some(0),
            );
            self.drain_events()?;
            let (symbol_names, symbol_count) = {
                let symbols_result = self
                    .debugger
                    .as_mut()
                    .and_then(|dbg| dbg.get_symbols(&regex).ok());
                match symbols_result {
                    Some(symbols) => (
                        Some(
                            symbols
                                .iter()
                                .map(|sym| sym.name.to_string())
                                .collect::<Vec<_>>(),
                        ),
                        Some(symbols.len()),
                    ),
                    None => (None, None),
                }
            };
            if let Some(names) = symbol_names {
                for name in names {
                    push_item(name);
                }
            }
            if let Some(count) = symbol_count {
                self.enqueue_progress_update(
                    progress_id.clone(),
                    Some(format!("Found {} symbols", count)),
                    Some(50),
                );
            } else {
                self.enqueue_progress_update(
                    progress_id.clone(),
                    Some("Symbol search failed".to_string()),
                    Some(50),
                );
            }
            self.enqueue_progress_end(progress_id, Some("Symbol search complete".to_string()));
            self.drain_events()?;
        }

        self.send_success_body(req, json!({ "targets": targets }))
    }
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
