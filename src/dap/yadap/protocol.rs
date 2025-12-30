use serde::{Deserialize, Serialize};
use serde_json::Value;

/// DAP request envelope.
#[derive(Debug, Deserialize)]
pub struct DapRequest {
    pub seq: i64,
    #[serde(rename = "type")]
    pub r#type: String,
    pub command: String,
    #[serde(default)]
    pub arguments: Value,
}

/// DAP response envelope.
///
/// Note: the DAP specification allows responses with no `body` field at all.
/// Using a `serde_json::Value` keeps the envelope stable and avoids type
/// inference issues around `None` bodies.
#[derive(Debug, Serialize)]
pub struct DapResponse {
    pub seq: i64,
    #[serde(rename = "type")]
    pub r#type: &'static str,
    pub request_seq: i64,
    pub success: bool,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
}

/// DAP event envelope.
#[derive(Debug, Serialize)]
pub struct DapEvent {
    pub seq: i64,
    #[serde(rename = "type")]
    pub r#type: &'static str,
    pub event: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
}

#[derive(Debug, Clone)]
pub enum InternalEvent {
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
    ProgressStart {
        progress_id: String,
        title: String,
        message: Option<String>,
        percentage: Option<u32>,
    },
    ProgressUpdate {
        progress_id: String,
        message: Option<String>,
        percentage: Option<u32>,
    },
    ProgressEnd {
        progress_id: String,
        message: Option<String>,
    },
    Invalidated {
        areas: Vec<String>,
    },
    Capabilities {
        capabilities: Value,
    },
}
