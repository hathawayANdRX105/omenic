//! Daemon wire protocol: newline-delimited JSON request/response frames.
//!
//! Every request from a client is a JSON object on its own line:
//!
//! ```json
//! {"id": "<client-chosen>", "type": "<command>", ...}
//! ```
//!
//! The daemon answers with a response on the same connection:
//!
//! ```json
//! {"id": "<same id>", "type": "response", "success": true, ...}
//! ```
//!
//! `id` is optional but strongly recommended so concurrent clients can
//! disambiguate replies.  When omitted, the daemon still echoes back the
//! field (or sets it to `null` in the response).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One client → daemon frame.
///
/// Build it from JSON or directly with [`Request::new`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub command: Command,
    #[serde(flatten)]
    pub params: Value,
}

impl Request {
    /// Start a request for `command`.
    pub fn new(command: Command) -> Self {
        Request {
            id: None,
            command,
            params: Value::Null,
        }
    }

    /// Set the request id.
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Replace the params with a JSON object built from `value`.
    pub fn with_params(mut self, params: Value) -> Self {
        self.params = params;
        self
    }
}

/// Closed set of commands the daemon understands.  Anything else is a
/// [`Protocol`](crate::DaemonError::Protocol) error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    /// `daemon.ping` — round-trip liveness check; returns `{ pong: true }`.
    #[serde(rename = "daemon.ping")]
    Ping,
    /// `daemon.shutdown` — clean exit.
    #[serde(rename = "daemon.shutdown")]
    Shutdown,
    /// `daemon.info` — `{ pid, uptime_ms, started_at_ms }`.
    #[serde(rename = "daemon.info")]
    Info,

    /// `session.create` — `{ id, title }` → `SessionSummary`.
    #[serde(rename = "session.create")]
    SessionCreate,
    /// `session.list` — `{ query, limit }` → `[SessionSummary]`.
    #[serde(rename = "session.list")]
    SessionList,
    /// `session.get` — `{ id }` → `SessionSummary | null`.
    #[serde(rename = "session.get")]
    SessionGet,
    /// `session.delete` — `{ id }` → `{ deleted: bool }`.
    #[serde(rename = "session.delete")]
    SessionDelete,
    /// `session.append` — `{ id, role, text }` → `{ seq, created_at_ms }`.
    #[serde(rename = "session.append")]
    SessionAppend,
    /// `session.load_messages` — `{ id, limit }` → `[SessionMessage]`.
    #[serde(rename = "session.load_messages")]
    SessionLoadMessages,
    /// `session.search` — `{ query, scope?: id, limit }` → `[SessionMessage]`.
    #[serde(rename = "session.search")]
    SessionSearch,

    /// `run.list` — `{ limit? }` → `[RunRecord]`.
    #[serde(rename = "run.list")]
    RunList,

    /// `worker.ping` — round-trip to the omp worker (spawns it on first use).
    #[serde(rename = "worker.ping")]
    WorkerPing,
    /// `worker.prompt` — `{ message }` → raw `rpc` response value.
    #[serde(rename = "worker.prompt")]
    WorkerPrompt,
    /// `worker.steer` — `{ message }` → raw `rpc` response value.
    #[serde(rename = "worker.steer")]
    WorkerSteer,
    /// `worker.abort` — `{}` → raw `rpc` response value.
    #[serde(rename = "worker.abort")]
    WorkerAbort,
    /// `worker.read_event` — `{}` → `WorkerEvent | null`.
    #[serde(rename = "worker.read_event")]
    WorkerReadEvent,
}

/// One daemon → client frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: ResponseKind,
    #[serde(default)]
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl Response {
    /// Successful response carrying an arbitrary JSON payload.
    pub fn ok(id: Option<&str>, data: Value) -> Self {
        Response {
            id: id.map(str::to_string),
            kind: ResponseKind::Response,
            success: true,
            error: None,
            data: Some(data),
        }
    }

    /// Failure response carrying a structured error.
    pub fn err(id: Option<&str>, error: ResponseError) -> Self {
        Response {
            id: id.map(str::to_string),
            kind: ResponseKind::Response,
            success: false,
            error: Some(error),
            data: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseKind {
    /// Successful response or structured failure response.
    #[serde(rename = "response")]
    Response,
    /// Server-issued push event (not currently used by the daemon; reserved).
    #[serde(rename = "event")]
    Event,
}

/// Structured error returned in `Response.error.code` / `message`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseError {
    /// Stable error code (e.g. `"session_not_found"`, `"protocol"`).
    pub code: String,
    /// Human-readable message (safe to log).
    pub message: String,
}

impl ResponseError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        ResponseError {
            code: code.into(),
            message: message.into(),
        }
    }
}
