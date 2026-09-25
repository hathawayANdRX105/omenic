//! ACP (Agent Client Protocol) transport: JSON-RPC 2.0 over NDJSON stdio.
//!
//! This is the protocol layer only — it owns the wire format and the read
//! thread. B2 implements [`AcpHandlers`] to fold streamed text into the
//! subagent result and to answer permission requests by policy.

use std::collections::HashMap;
use std::fmt;
use std::io::{self, BufReader, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use wire::{jsonrpc, stdio};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Protocol version advertised in `initialize`.
pub const PROTOCOL_VERSION: &str = "1";

pub const JSONRPC: &str = "2.0";

pub const METHOD_INITIALIZE: &str = "initialize";
pub const METHOD_SESSION_NEW: &str = "session/new";
pub const METHOD_SESSION_PROMPT: &str = "session/prompt";
pub const METHOD_SESSION_CANCEL: &str = "session/cancel";
pub const NOTIFICATION_SESSION_UPDATE: &str = "session/update";
pub const REQUEST_PERMISSION: &str = "session/requestPermission";

/// Failure raised by the ACP transport. Hand-rolled because this layer may
/// not pull in error crates.
#[derive(Debug)]
pub enum AcpError {
    /// stdio failure.
    Io(io::Error),
    /// A message could not be (de)serialized.
    Serialize(serde_json::Error),
    /// The agent's reply could not be routed, or was an error response.
    Protocol(String, Option<Value>),
    /// The client was closed while a request was in flight.
    ChannelClosed,
}

impl fmt::Display for AcpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AcpError::Io(e) => write!(f, "ACP io error: {e}"),
            AcpError::Serialize(e) => write!(f, "ACP serde error: {e}"),
            AcpError::Protocol(msg, raw) => {
                write!(f, "ACP protocol error: {msg}")?;
                match raw {
                    Some(raw) => write!(f, " (raw: {raw})"),
                    None => Ok(()),
                }
            }
            AcpError::ChannelClosed => write!(f, "ACP response channel closed"),
        }
    }
}

impl std::error::Error for AcpError {}

impl From<io::Error> for AcpError {
    fn from(e: io::Error) -> Self {
        AcpError::Io(e)
    }
}

impl From<serde_json::Error> for AcpError {
    fn from(e: serde_json::Error) -> Self {
        AcpError::Serialize(e)
    }
}

/// Why the agent stopped producing output. Defaults to `"end_turn"`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StopReason(String);

impl StopReason {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for StopReason {
    fn default() -> Self {
        StopReason("end_turn".to_string())
    }
}

/// One text chunk streamed by the agent.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TextContent {
    pub text: String,
}

/// Body of `session/prompt`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PromptRequest {
    pub prompt: Vec<TextContent>,
}

/// Body of the `session/prompt` response.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptResponse {
    pub stop_reason: Option<StopReason>,
}

/// Client capabilities sent in `initialize`. omenic advertises none yet.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ClientCapabilities {}

/// Body of `initialize`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeRequest {
    pub protocol_version: String,
    pub client_capabilities: ClientCapabilities,
}

impl InitializeRequest {
    pub fn new() -> Self {
        InitializeRequest {
            protocol_version: PROTOCOL_VERSION.to_string(),
            client_capabilities: ClientCapabilities {},
        }
    }
}

impl Default for InitializeRequest {
    fn default() -> Self {
        Self::new()
    }
}

/// Body of the `initialize` response. Agent capabilities stay raw: agents
/// differ widely in what they advertise and we only forward them.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResponse {
    pub protocol_version: String,
    #[serde(default)]
    pub agent_capabilities: Value,
    #[serde(default)]
    pub auth_methods: Vec<String>,
}

/// Body of `session/new`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSessionRequest {
    #[serde(default)]
    pub mcp_servers: Vec<Value>,
    #[serde(default)]
    pub cwd: String,
}

/// Body of the `session/new` response.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSessionResponse {
    pub session_id: String,
}

/// Discriminated `session/update` body. Only `agentMessageChunk` is consumed;
/// every other update type is ignored without failing the stream.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SessionUpdate {
    AgentMessageChunk {
        content: Vec<TextContent>,
    },
    /// ponytail: catches every other update type so an unknown one never
    /// kills the stream.
    #[serde(other)]
    Other,
}

/// `session/update` notification envelope.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionUpdateEnvelope {
    pub session_id: String,
    pub update: SessionUpdate,
}

/// One selectable option in a permission request.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionOption {
    pub option_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Body of `session/requestPermission`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestPermissionRequest {
    #[serde(default)]
    pub session_id: String,
    pub permission_id: String,
    pub options: Vec<PermissionOption>,
}

/// Outcome sent back for a permission request.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PermissionOutcome {
    #[serde(rename_all = "camelCase")]
    Allow {
        option_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        modified_call: Option<Value>,
    },
    Deny,
}

/// Answer to a permission request.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RequestPermissionResponse {
    pub outcome: PermissionOutcome,
}

/// Hooks the ACP provider (B2) plugs into. Every method is synchronous: the
/// transport is std-only, so handlers run on the read thread.
pub trait AcpHandlers: Send + Sync {
    /// Called for each text chunk the agent streams in a session.
    fn on_message_chunk(&self, session_id: &str, text: &str);
    /// Called when the agent asks permission; the returned outcome is sent
    /// back to the agent with the same request id.
    fn on_permission(&self, request: RequestPermissionRequest) -> RequestPermissionResponse;
}

/// stdio endpoints of an agent child process. In production this is
/// `(ChildStdin, ChildStdout)`; tests drive it with in-memory pipes.
pub struct AcpTransport<W: Write + Send + 'static, R: Read + Send + 'static> {
    writer: Mutex<W>,
    /// Taken once by the read thread in [`AcpClient::new`].
    reader: Mutex<Option<BufReader<R>>>,
}

/// JSON-RPC 2.0 client driving an agent child process over stdio.
pub struct AcpClient<W: Write + Send + 'static, R: Read + Send + 'static> {
    transport: Arc<AcpTransport<W, R>>,
    /// Reply channels for in-flight requests, keyed by request id.
    pending: Arc<Mutex<HashMap<u64, SyncSender<Value>>>>,
    next_id: AtomicU64,
}

impl<W: Write + Send + 'static, R: Read + Send + 'static> AcpClient<W, R> {
    /// Spawn the read thread and start dispatching inbound messages.
    pub fn new(writer: W, reader: R, handlers: Arc<dyn AcpHandlers>) -> Self {
        let transport = Arc::new(AcpTransport {
            writer: Mutex::new(writer),
            reader: Mutex::new(Some(BufReader::new(reader))),
        });
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let client = AcpClient {
            transport: transport.clone(),
            pending: pending.clone(),
            next_id: AtomicU64::new(1),
        };
        let reader = transport
            .reader
            .lock()
            .unwrap()
            .take()
            .expect("reader taken once");
        thread::spawn(move || serve(reader, transport, pending, handlers));
        client
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// `initialize` — the first request on a fresh transport.
    pub fn initialize(&self) -> Result<InitializeResponse, AcpError> {
        let value = self.request(METHOD_INITIALIZE, InitializeRequest::new())?;
        Ok(serde_json::from_value(value)?)
    }

    /// `session/new` — start a session rooted at `cwd`.
    pub fn new_session(&self, cwd: &str) -> Result<NewSessionResponse, AcpError> {
        let body = NewSessionRequest {
            mcp_servers: Vec::new(),
            cwd: cwd.to_string(),
        };
        let value = self.request(METHOD_SESSION_NEW, body)?;
        Ok(serde_json::from_value(value)?)
    }

    /// `session/prompt` — block until the agent finishes the turn.
    pub fn prompt(&self, session_id: &str, text: &str) -> Result<PromptResponse, AcpError> {
        let mut params = serde_json::to_value(PromptRequest {
            prompt: vec![TextContent {
                text: text.to_string(),
            }],
        })?;
        params["sessionId"] = Value::String(session_id.to_string());
        let value = self.request_value(METHOD_SESSION_PROMPT, params)?;
        Ok(serde_json::from_value(value)?)
    }

    /// `session/cancel` — a notification, so it takes no reply.
    pub fn cancel(&self, session_id: &str) -> Result<(), AcpError> {
        self.transport.write_json(&jsonrpc::notification_value(
            METHOD_SESSION_CANCEL,
            json!({ "sessionId": session_id }),
        ))
    }

    /// Drop all pending reply channels. In-flight requests then fail with
    /// [`AcpError::ChannelClosed`].
    pub fn close(&self) -> Result<(), AcpError> {
        self.pending.lock().unwrap().clear();
        Ok(())
    }

    fn request<B: Serialize>(&self, method: &str, body: B) -> Result<Value, AcpError> {
        self.request_value(method, serde_json::to_value(body)?)
    }

    fn request_value(&self, method: &str, params: Value) -> Result<Value, AcpError> {
        let id = self.next_id();
        let (tx, rx) = mpsc::sync_channel::<Value>(1);
        self.pending.lock().unwrap().insert(id, tx);

        self.transport
            .write_json(&jsonrpc::request_value(id, method, params))?;

        let value = rx.recv().map_err(|_| AcpError::ChannelClosed)?;
        if let Some(err) = value.get("error") {
            return Err(AcpError::Protocol(
                "agent returned an error response".to_string(),
                Some(err.clone()),
            ));
        }
        value
            .get("result")
            .cloned()
            .ok_or_else(|| AcpError::Protocol("response without a result".to_string(), Some(value)))
    }
}

impl<W: Write + Send + 'static, R: Read + Send + 'static> AcpTransport<W, R> {
    /// Frame one JSON object per line onto the agent's stdin.
    fn write_json(&self, value: &Value) -> Result<(), AcpError> {
        let line = jsonrpc::encode_line(value);
        stdio::write_json_line(&mut *self.writer.lock().unwrap(), &line)?;
        Ok(())
    }
}

/// Read thread: dispatch each NDJSON line to its handler.
///
/// Responses to our requests are matched by `id` against the pending table,
/// notifications are routed to [`AcpHandlers`], and agent requests
/// (`session/requestPermission`) are answered on stdin with the same id.
fn serve<W: Write + Send + 'static, R: Read + Send + 'static>(
    mut reader: BufReader<R>,
    transport: Arc<AcpTransport<W, R>>,
    pending: Arc<Mutex<HashMap<u64, SyncSender<Value>>>>,
    handlers: Arc<dyn AcpHandlers>,
) {
    let mut line = String::new();
    loop {
        line.clear();
        match stdio::read_line(&mut reader, &mut line) {
            Ok(false) => break, // EOF
            Ok(true) => {}
            Err(_) => break,
        }
        // A malformed line is skipped, never fatal: the agent may emit logs.
        let value = match jsonrpc::decode_line(&line) {
            Some(v) => v,
            None => continue,
        };
        match jsonrpc::classify(&value) {
            jsonrpc::FrameKind::Request { id, method, value } => {
                handle_request(Value::from(id), method, value, &transport, &handlers)
            }
            jsonrpc::FrameKind::Response { id, value } => {
                // Unknown ids are ignored: they belong to requests we no
                // longer care about, and must never panic the thread.
                if let Some(tx) = pending.lock().unwrap().remove(&id) {
                    let _ = tx.send(value.clone());
                }
            }
            jsonrpc::FrameKind::Notification { method, value } => {
                handle_notification(method, value, &handlers)
            }
            jsonrpc::FrameKind::Ignored => {}
        }
    }
}

fn handle_request<W: Write + Send + 'static, R: Read + Send + 'static>(
    id: Value,
    method: &str,
    value: &Value,
    transport: &AcpTransport<W, R>,
    handlers: &Arc<dyn AcpHandlers>,
) {
    if method != REQUEST_PERMISSION {
        return;
    }
    let params = value.get("params").cloned().unwrap_or(Value::Null);
    let request = match serde_json::from_value::<RequestPermissionRequest>(params) {
        Ok(request) => request,
        Err(_) => return,
    };
    let outcome = handlers.on_permission(request);
    let outcome = serde_json::to_value(outcome).unwrap_or(Value::Null);
    let _ = transport.write_json(&json!({
        "jsonrpc": JSONRPC,
        "id": id,
        "result": outcome,
    }));
}

fn handle_notification(method: &str, value: &Value, handlers: &Arc<dyn AcpHandlers>) {
    if method != NOTIFICATION_SESSION_UPDATE {
        return;
    }
    let params = value.get("params").cloned().unwrap_or(Value::Null);
    let envelope = match serde_json::from_value::<SessionUpdateEnvelope>(params) {
        Ok(envelope) => envelope,
        Err(_) => return,
    };
    match envelope.update {
        SessionUpdate::AgentMessageChunk { content } => {
            for chunk in content {
                handlers.on_message_chunk(&envelope.session_id, &chunk.text);
            }
        }
        SessionUpdate::Other => {}
    }
}
