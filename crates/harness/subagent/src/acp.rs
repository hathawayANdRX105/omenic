//! ACP (Agent Client Protocol) transport: JSON-RPC 2.0 over NDJSON stdio.
//!
//! This is the protocol layer only — it owns the wire format and the read
//! thread. B2 implements [`AcpHandlers`] to fold streamed text into the
//! subagent result and to answer permission requests by policy.

use std::collections::HashMap;
use std::fmt;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Protocol version advertised in `initialize`.
pub const PROTOCOL_VERSION: &str = "1";

const JSONRPC: &str = "2.0";

const METHOD_INITIALIZE: &str = "initialize";
const METHOD_SESSION_NEW: &str = "session/new";
const METHOD_SESSION_PROMPT: &str = "session/prompt";
const METHOD_SESSION_CANCEL: &str = "session/cancel";
const NOTIFICATION_SESSION_UPDATE: &str = "session/update";
const REQUEST_PERMISSION: &str = "session/requestPermission";

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
pub struct PromptResponse {
    pub stop_reason: Option<StopReason>,
}

/// Client capabilities sent in `initialize`. omenic advertises none yet.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ClientCapabilities {}

/// Body of `initialize`.
#[derive(Clone, Debug, Deserialize, Serialize)]
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
struct SessionUpdateEnvelope {
    session_id: String,
    update: SessionUpdate,
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
        self.transport.write_json(&json!({
            "jsonrpc": JSONRPC,
            "method": METHOD_SESSION_CANCEL,
            "params": { "sessionId": session_id },
        }))
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

        self.transport.write_json(&json!({
            "jsonrpc": JSONRPC,
            "id": id,
            "method": method,
            "params": params,
        }))?;

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
        let mut line = serde_json::to_string(value)?;
        line.push('\n');
        let mut writer = self.writer.lock().unwrap();
        writer.write_all(line.as_bytes())?;
        writer.flush()?;
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
        match reader.read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(_) => break,
        }
        if line.trim().is_empty() {
            continue;
        }
        // A malformed line is skipped, never fatal: the agent may emit logs.
        let value = match serde_json::from_str::<Value>(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        match (
            value.get("id").cloned(),
            value.get("method").and_then(Value::as_str),
        ) {
            (Some(id), Some(method)) => handle_request(id, method, &value, &transport, &handlers),
            (Some(id), None) => {
                if let Some(id) = id.as_u64() {
                    // Unknown ids are ignored: they belong to requests we no
                    // longer care about, and must never panic the thread.
                    if let Some(tx) = pending.lock().unwrap().remove(&id) {
                        let _ = tx.send(value);
                    }
                }
            }
            (None, Some(method)) => handle_notification(method, &value, &handlers),
            (None, None) => {}
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::Receiver;

    /// Byte pipe in one direction, backed by an mpsc channel so the reader
    /// blocks instead of busy-waiting. Dropping the sender reads as EOF.
    struct PipeRead {
        rx: Receiver<Vec<u8>>,
        backlog: Vec<u8>,
    }

    impl Read for PipeRead {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.backlog.is_empty() {
                match self.rx.recv() {
                    Ok(chunk) => self.backlog.extend_from_slice(&chunk),
                    Err(_) => return Ok(0),
                }
            }
            let n = self.backlog.len().min(buf.len());
            buf[..n].copy_from_slice(&self.backlog[..n]);
            self.backlog.drain(..n);
            Ok(n)
        }
    }

    struct PipeWrite {
        tx: SyncSender<Vec<u8>>,
    }

    impl Write for PipeWrite {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.tx
                .send(buf.to_vec())
                .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "pipe closed"))?;
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Records streamed chunks and grants the permission option it recognizes.
    struct TestHandlers {
        chunks: Mutex<Vec<(String, String)>>,
        allowed_option: String,
    }

    impl AcpHandlers for TestHandlers {
        fn on_message_chunk(&self, session_id: &str, text: &str) {
            self.chunks
                .lock()
                .unwrap()
                .push((session_id.to_string(), text.to_string()));
        }

        fn on_permission(&self, request: RequestPermissionRequest) -> RequestPermissionResponse {
            let granted = request
                .options
                .iter()
                .any(|o| o.option_id == self.allowed_option);
            let outcome = if granted {
                PermissionOutcome::Allow {
                    option_id: self.allowed_option.clone(),
                    modified_call: None,
                }
            } else {
                PermissionOutcome::Deny
            };
            RequestPermissionResponse { outcome }
        }
    }

    /// Fake agent: answers the client's requests, streams chunks per prompt,
    /// and records permission answers it receives back.
    struct FakeAgent {
        requests: Mutex<Vec<Value>>,
        permission_answers: Mutex<Vec<Value>>,
        /// Reply to `initialize` with an error instead of a result.
        fail_initialize: std::sync::atomic::AtomicBool,
        /// Record `session/new` but never answer it.
        silent_new_session: std::sync::atomic::AtomicBool,
    }

    impl FakeAgent {
        fn send(&self, tx: &SyncSender<Vec<u8>>, value: &Value) {
            let mut line = serde_json::to_string(value).unwrap();
            line.push('\n');
            tx.send(line.into_bytes()).unwrap();
        }

        fn handle_line(&self, line: &str, tx: &SyncSender<Vec<u8>>) {
            let value: Value = match serde_json::from_str(line) {
                Ok(value) => value,
                Err(_) => return,
            };
            let id = value.get("id").cloned();
            match (id, value.get("method").and_then(Value::as_str)) {
                (Some(id), Some(METHOD_INITIALIZE)) => {
                    self.requests.lock().unwrap().push(value.clone());
                    if self.fail_initialize.load(Ordering::Relaxed) {
                        self.send(
                            tx,
                            &json!({
                                "jsonrpc": JSONRPC,
                                "id": id,
                                "error": { "code": -32000, "message": "agent refused" }
                            }),
                        );
                        return;
                    }
                    self.send(
                        tx,
                        &json!({
                            "jsonrpc": JSONRPC,
                            "id": id,
                            "result": {
                                "protocolVersion": PROTOCOL_VERSION,
                                "agentCapabilities": {},
                                "authMethods": [],
                            }
                        }),
                    );
                }
                (Some(id), Some(METHOD_SESSION_NEW)) => {
                    self.requests.lock().unwrap().push(value.clone());
                    if self.silent_new_session.load(Ordering::Relaxed) {
                        return;
                    }
                    self.send(
                        tx,
                        &json!({
                            "jsonrpc": JSONRPC,
                            "id": id,
                            "result": { "sessionId": "sess-1" }
                        }),
                    );
                }
                (Some(id), Some(METHOD_SESSION_PROMPT)) => {
                    self.requests.lock().unwrap().push(value.clone());
                    let session_id = value["params"]["sessionId"].as_str().unwrap_or("");
                    for i in 0..2usize {
                        self.send(
                            tx,
                            &json!({
                                "jsonrpc": JSONRPC,
                                "method": NOTIFICATION_SESSION_UPDATE,
                                "params": {
                                    "sessionId": session_id,
                                    "update": {
                                        "type": "agentMessageChunk",
                                        "content": [{ "text": format!("chunk-{i}") }],
                                    }
                                }
                            }),
                        );
                    }
                    self.send(
                        tx,
                        &json!({
                            "jsonrpc": JSONRPC,
                            "id": id,
                            "result": { "stopReason": "end_turn" }
                        }),
                    );
                }
                (Some(id), Some(REQUEST_PERMISSION)) => {
                    self.permission_answers.lock().unwrap().push(json!({
                        "id": id,
                        "result": value.get("result").cloned().unwrap_or(Value::Null),
                    }));
                }
                _ => {}
            }
        }
    }

    /// Wire a client to a fake agent on its own thread; also hand back the
    /// agent-to-client sender so a test can inject raw agent traffic.
    fn harness(
        allowed_option: &str,
    ) -> (
        AcpClient<PipeWrite, PipeRead>,
        Arc<FakeAgent>,
        Arc<TestHandlers>,
        SyncSender<Vec<u8>>,
    ) {
        let (client_tx, client_rx) = mpsc::sync_channel::<Vec<u8>>(16);
        let (agent_tx, agent_rx) = mpsc::sync_channel::<Vec<u8>>(16);
        let agent = Arc::new(FakeAgent {
            requests: Mutex::new(Vec::new()),
            permission_answers: Mutex::new(Vec::new()),
            fail_initialize: std::sync::atomic::AtomicBool::new(false),
            silent_new_session: std::sync::atomic::AtomicBool::new(false),
        });
        let handlers = Arc::new(TestHandlers {
            chunks: Mutex::new(Vec::new()),
            allowed_option: allowed_option.to_string(),
        });

        let agent_thread = agent.clone();
        let agent_reply = agent_tx.clone();
        thread::spawn(move || {
            let mut reader = BufReader::new(PipeRead {
                rx: client_rx,
                backlog: Vec::new(),
            });
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Err(_) => break,
                    _ => {
                        if line.trim().is_empty() {
                            continue;
                        }
                        agent_thread.handle_line(line.trim_end(), &agent_reply);
                    }
                }
            }
        });

        let client = AcpClient::new(
            PipeWrite { tx: client_tx },
            PipeRead {
                rx: agent_rx,
                backlog: Vec::new(),
            },
            handlers.clone(),
        );
        (client, agent, handlers, agent_tx)
    }

    fn settle() {
        std::thread::sleep(std::time::Duration::from_millis(150));
    }

    #[test]
    fn initialize_new_session_and_prompt_round_trip() {
        let (client, agent, handlers, _tx) = harness("allow");
        let init = client.initialize().expect("initialize");
        assert_eq!(init.protocol_version, PROTOCOL_VERSION);

        let session = client.new_session("/tmp").expect("session/new");
        assert_eq!(session.session_id, "sess-1");

        let response = client.prompt("sess-1", "hello").expect("session/prompt");
        assert_eq!(
            response.stop_reason.map(|r| r.as_str().to_string()),
            Some("end_turn".to_string())
        );

        settle();
        // The agent saw our three requests, in order, with our prompt text.
        let requests = agent.requests.lock().unwrap();
        let methods: Vec<_> = requests
            .iter()
            .map(|v| v["method"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(methods, vec!["initialize", "session/new", "session/prompt"]);
        assert_eq!(
            requests[2]["params"]["prompt"][0]["text"].as_str(),
            Some("hello")
        );

        // And both streamed chunks reached the handler, keyed by session.
        let chunks = handlers.chunks.lock().unwrap();
        assert_eq!(
            chunks.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>(),
            vec!["chunk-0", "chunk-1"]
        );
        assert_eq!(chunks[0].0, "sess-1");
    }

    #[test]
    fn cancel_is_a_notification_without_id() {
        let (client, agent, _handlers, _tx) = harness("allow");
        client.initialize().unwrap();
        client.new_session("/tmp").unwrap();
        client.cancel("sess-1").expect("session/cancel");
        settle();

        // The agent saw initialize and session/new; cancel carries no id so it
        // never reaches the request log and never expects a reply.
        let requests = agent.requests.lock().unwrap();
        let methods: Vec<_> = requests
            .iter()
            .map(|v| v["method"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(methods, vec!["initialize", "session/new"]);
    }

    #[test]
    fn permission_request_is_answered_with_option_id() {
        let (client, agent, _handlers, agent_tx) = harness("allow-full");
        // The agent asks for permission; the client must answer it itself.
        agent_tx
            .send(
                format!(
                    "{{\"jsonrpc\":\"{JSONRPC}\",\"id\":42,\"method\":\"{REQUEST_PERMISSION}\",\"params\":{{\"sessionId\":\"sess-1\",\"permissionId\":\"shell\",\"options\":[{{\"optionId\":\"allow-full\",\"title\":\"Allow\"}},{{\"optionId\":\"deny\",\"title\":\"Deny\"}}]}}}}\n"
                )
                .into_bytes(),
            )
            .unwrap();
        settle();
        drop(client);
        settle();

        // The reply reached the agent with the same id and the granted option.
        let answers = agent.permission_answers.lock().unwrap();
        assert_eq!(answers.len(), 1, "agent received no permission reply");
        assert_eq!(answers[0]["id"], 42);
        assert_eq!(answers[0]["result"]["outcome"]["type"], "allow");
        assert_eq!(answers[0]["result"]["outcome"]["optionId"], "allow-full");
    }

    #[test]
    fn permission_is_denied_when_option_is_not_allowed() {
        let (client, agent, _handlers, agent_tx) = harness("allow-full");
        agent_tx
            .send(
                format!(
                    "{{\"jsonrpc\":\"{JSONRPC}\",\"id\":7,\"method\":\"{REQUEST_PERMISSION}\",\"params\":{{\"sessionId\":\"sess-1\",\"permissionId\":\"shell\",\"options\":[{{\"optionId\":\"deny\",\"title\":\"Deny\"}}]}}}}\n"
                )
                .into_bytes(),
            )
            .unwrap();
        settle();
        drop(client);
        settle();

        let answers = agent.permission_answers.lock().unwrap();
        assert_eq!(answers[0]["result"]["outcome"]["type"], "deny");
    }

    #[test]
    fn malformed_and_unknown_lines_do_not_kill_the_reader() {
        let (client, agent, _handlers, agent_tx) = harness("allow");
        // Garbage line, then a notification the client does not know, then a
        // well-formed update: the reader must survive all three and still
        // deliver the real chunk.
        agent_tx.send(b"this is not json\n".to_vec()).unwrap();
        agent_tx
            .send(
                format!(
                    "{{\"jsonrpc\":\"{JSONRPC}\",\"method\":\"session/someFutureNotification\",\"params\":{{}}}}\n"
                )
                .into_bytes(),
            )
            .unwrap();
        agent_tx
            .send(
                format!(
                    "{{\"jsonrpc\":\"{JSONRPC}\",\"method\":\"{NOTIFICATION_SESSION_UPDATE}\",\"params\":{{\"sessionId\":\"sess-9\",\"update\":{{\"type\":\"agentMessageChunk\",\"content\":[{{\"text\":\"survivor\"}}]}}}}}}\n"
                )
                .into_bytes(),
            )
            .unwrap();
        // Now a round trip to prove the reader thread is still dispatching.
        client.initialize().expect("initialize still works");
        settle();

        let requests = agent.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[test]
    fn unknown_pending_id_is_ignored_without_panicking() {
        let (client, _agent, _handlers, agent_tx) = harness("allow");
        // A reply to an id we never asked for must not panic the read thread.
        agent_tx
            .send(
                format!("{{\"jsonrpc\":\"{JSONRPC}\",\"id\":999,\"result\":{{}}}}\n").into_bytes(),
            )
            .unwrap();
        settle();
        // The reader is still alive: a real round trip completes.
        client.initialize().expect("initialize still works");
    }

    #[test]
    fn close_fails_in_flight_request_with_channel_closed() {
        let (client, agent, _handlers, _tx) = harness("allow");
        // The agent accepts session/new but never answers it.
        agent.silent_new_session.store(true, Ordering::Relaxed);
        let client = Arc::new(client);
        let pending = {
            let client = client.clone();
            thread::spawn(move || client.new_session("/tmp"))
        };
        settle(); // the request is now in flight with no reply coming
        client.close().unwrap();
        match pending.join().unwrap() {
            Err(AcpError::ChannelClosed) => {}
            other => panic!("expected ChannelClosed, got {other:?}"),
        }
    }

    #[test]
    fn agent_error_response_becomes_protocol_error() {
        let (client, agent, _handlers, _tx) = harness("allow");
        agent.fail_initialize.store(true, Ordering::Relaxed);
        match client.initialize() {
            Err(AcpError::Protocol(_, _)) => {}
            other => panic!("expected Protocol error, got {other:?}"),
        }
    }

    #[test]
    fn session_update_envelope_renames_fields() {
        let raw = serde_json::json!({
            "sessionId": "s",
            "update": { "type": "agentMessageChunk", "content": [{ "text": "hi" }] }
        });
        let env: SessionUpdateEnvelope = serde_json::from_value(raw).unwrap();
        match env.update {
            SessionUpdate::AgentMessageChunk { content } => {
                assert_eq!(content[0].text, "hi");
                assert_eq!(env.session_id, "s");
            }
            SessionUpdate::Other => panic!("parsed as Other"),
        }
    }

    #[test]
    fn unknown_session_update_type_is_ignored() {
        let raw = serde_json::json!({
            "sessionId": "s",
            "update": { "type": "someFutureKind", "content": [] }
        });
        let env: SessionUpdateEnvelope = serde_json::from_value(raw).unwrap();
        assert!(matches!(env.update, SessionUpdate::Other));
    }
}
