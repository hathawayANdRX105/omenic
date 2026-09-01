//! MCP (Model Context Protocol) client: JSON-RPC 2.0 over stdio.
//!
//! Default off — nothing is spawned unless `[[mcp.servers]]` is configured.
//! An MCP server is a child process; its `tools/list` entries map onto the
//! built-in [`tools::Tool`] trait so the agent loop can't tell them apart from
//! native tools.
//!
//! Layering: JSON-RPC framing is free functions over an [`McpTransport`], so
//! the protocol is testable without spawning anything. [`StdioTransport`] is
//! the only real implementation.

pub mod tool;

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use adaptor::ToolDef;
use config::McpServerConfig;
use serde_json::{Value, json};
use tools::{Tool, ToolError};

pub use tool::{McpTool, ToolMeta};

/// MCP basic spec revision this client implements.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Per-request budget and abort poll interval. Matches the built-in tools'
/// subprocess timeout so a hung server can't wedge the agent loop.
pub const MCP_TIMEOUT: Duration = Duration::from_secs(30);
pub const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Everything that can go wrong talking to an MCP server.
#[derive(Debug)]
pub enum McpError {
    /// The child process could not be started.
    Spawn(String),
    /// Pipe broken, unreadable, or closed mid-session.
    Transport(String),
    /// Well-formed transport, malformed MCP.
    Protocol(String),
    /// JSON-RPC `error` object from the server.
    Server { code: i64, message: String },
    /// The tool ran and reported failure (`isError: true`).
    Tool(String),
    /// No response within [`MCP_TIMEOUT`].
    Timeout,
    /// The caller's abort signal fired.
    Aborted,
}

impl std::fmt::Display for McpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpError::Spawn(m) => write!(f, "mcp: failed to start server: {m}"),
            McpError::Transport(m) => write!(f, "mcp: transport error: {m}"),
            McpError::Protocol(m) => write!(f, "mcp: protocol error: {m}"),
            McpError::Server { code, message } => write!(f, "mcp: server error {code}: {message}"),
            McpError::Tool(m) => write!(f, "{m}"),
            McpError::Timeout => write!(f, "mcp: server did not respond in time"),
            McpError::Aborted => write!(f, "aborted"),
        }
    }
}

impl std::error::Error for McpError {}

impl From<McpError> for ToolError {
    fn from(e: McpError) -> ToolError {
        ToolError::Message(e.to_string())
    }
}
/// One request/response channel to an MCP server.
///
/// Implementations must serialize concurrent callers: `roundtrip` is called
/// from tool executions on arbitrary threads.
pub trait McpTransport: Send + Sync {
    /// Send one framed JSON-RPC request and return the reply line for `id`.
    /// Lines without a matching id (server notifications, or a response from
    /// a previous roundtrip) must be consumed silently and never returned.
    /// Must poll `signal` and give up after [`MCP_TIMEOUT`].
    fn roundtrip(&self, id: u64, line: &str, signal: &AtomicBool) -> Result<String, McpError>;
    /// Send a notification (no `id`, no reply expected).
    fn notify(&self, line: &str) -> Result<(), McpError>;
    /// Next monotonic request id.
    fn next_id(&self) -> u64;
}

/// Serialize a JSON-RPC 2.0 request.
pub fn request_line(id: u64, method: &str, params: &Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
}

/// Serialize a JSON-RPC 2.0 notification.
pub fn notification_line(method: &str, params: &Value) -> String {
    json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string()
}

/// Extract the `result` of a response; `error` and id mismatch become [`McpError`].
pub fn parse_response(line: &str, id: u64) -> Result<Value, McpError> {
    let mut v: Value = serde_json::from_str(line)
        .map_err(|e| McpError::Protocol(format!("response is not valid JSON: {e}")))?;
    match v.get("id").and_then(Value::as_u64) {
        Some(got) if got == id => {}
        Some(got) => {
            return Err(McpError::Protocol(format!(
                "response id {got} does not match request id {id}"
            )));
        }
        None => return Err(McpError::Protocol("response has no `id`".into())),
    }
    if let Some(err) = v.get("error") {
        return Err(McpError::Server {
            code: err.get("code").and_then(Value::as_i64).unwrap_or(0),
            message: err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error")
                .to_string(),
        });
    }
    Ok(v.get_mut("result")
        .map(std::mem::take)
        .unwrap_or(Value::Null))
}

/// One full request: allocate an id, frame it, await the reply, unwrap `result`.
pub fn request(
    transport: &dyn McpTransport,
    method: &str,
    params: &Value,
    signal: &AtomicBool,
) -> Result<Value, McpError> {
    let id = transport.next_id();
    let line = transport.roundtrip(id, &request_line(id, method, params), signal)?;
    parse_response(&line, id)
}

/// MCP handshake: `initialize`, then the `notifications/initialized` ack.
pub fn initialize(transport: &dyn McpTransport, signal: &AtomicBool) -> Result<Value, McpError> {
    let params = json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {},
        "clientInfo": {"name": "omenic", "version": env!("CARGO_PKG_VERSION")},
    });
    let result = request(transport, "initialize", &params, signal)?;
    transport.notify(&notification_line("notifications/initialized", &json!({})))?;
    Ok(result)
}
/// `tools/list` → tool metadata. Entries without a usable `name` are skipped
/// rather than failing the whole server.
///
/// Follows `nextCursor` until the server returns null, so paginated servers
/// don't silently drop everything past page one.
pub fn list_tools(
    transport: &dyn McpTransport,
    server: &str,
    signal: &AtomicBool,
) -> Result<Vec<ToolMeta>, McpError> {
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let params = match &cursor {
            Some(c) => json!({ "cursor": c }),
            None => json!({}),
        };
        let result = request(transport, "tools/list", &params, signal)?;
        let items = result
            .get("tools")
            .and_then(Value::as_array)
            .ok_or_else(|| McpError::Protocol("tools/list result has no `tools` array".into()))?;
        for t in items {
            let Some(remote) = t.get("name").and_then(Value::as_str) else {
                continue;
            };
            if remote.is_empty() {
                continue;
            }
            out.push(ToolMeta {
                name: format!("mcp__{server}__{remote}"),
                remote: remote.to_string(),
                description: t
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                // Absent schema means "no arguments", not a broken tool.
                parameters: t
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
            });
        }
        cursor = match result.get("nextCursor") {
            Some(Value::Null) | None => break,
            Some(v) => match v.as_str() {
                Some("") | None => break,
                Some(s) => Some(s.to_string()),
            },
        };
    }
    Ok(out)
}
/// `tools/call` → flattened text content.
///
/// Per spec the result carries a `content` array of typed parts plus an
/// `isError` flag; text parts are joined and `isError` becomes [`McpError::Tool`]
/// so the agent loop surfaces it like any other tool failure.
pub fn call_tool(
    transport: &dyn McpTransport,
    remote: &str,
    args: &Value,
    signal: &AtomicBool,
) -> Result<String, McpError> {
    let params = json!({
        "name": remote,
        // Servers reject a non-object `arguments`; normalize null/scalars away.
        "arguments": if args.is_object() { args.clone() } else { json!({}) },
    });
    let result = request(transport, "tools/call", &params, signal)?;
    let text = flatten_content(&result);
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        return Err(McpError::Tool(if text.is_empty() {
            format!("mcp tool `{remote}` failed")
        } else {
            text
        }));
    }
    Ok(text)
}

/// Join the text parts of a `tools/call` result. Non-text parts are rendered as
/// compact JSON so nothing is silently dropped.
fn flatten_content(result: &Value) -> String {
    let Some(items) = result.get("content").and_then(Value::as_array) else {
        return String::new();
    };
    items
        .iter()
        .map(|part| match part.get("text").and_then(Value::as_str) {
            Some(t) => t.to_string(),
            None => part.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Take a mutex, tolerating poisoning.
///
/// ponytail: `std::sync::Mutex` because this crate takes no new dependencies;
/// a panic while holding the lock only ever leaves protected data mid-write on
/// a connection we are about to error out of, so recovering the guard beats
/// propagating a poison panic into the agent loop.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Write one newline-delimited JSON-RPC message and flush it.
fn write_line(stdin: &mut ChildStdin, line: &str) -> Result<(), McpError> {
    stdin
        .write_all(line.as_bytes())
        .and_then(|()| stdin.write_all(b"\n"))
        .and_then(|()| stdin.flush())
        .map_err(|e| McpError::Transport(format!("write to server stdin failed: {e}")))
}

/// Stdio transport: a child process spoken to over line-delimited JSON-RPC.
///
/// stdout is drained by a reader thread into a channel, so a chatty server can
/// never fill the pipe and deadlock. The write half plus the receiver sit behind
/// one mutex: MCP allows pipelining, we don't need it, and one lock keeps request
/// and reply from interleaving across threads.
///
/// ponytail: single mutex serializes all calls to one server. Add id-keyed
/// demultiplexing only if concurrent tool calls to the same server show up hot.
pub struct StdioTransport {
    io: Mutex<Io>,
    child: Mutex<Child>,
    next: AtomicU64,
}

struct Io {
    stdin: ChildStdin,
    replies: Receiver<String>,
}

impl StdioTransport {
    /// Spawn `command` with pipes wired up. stderr is inherited so server
    /// diagnostics land in the host's log instead of filling an unread pipe.
    pub fn spawn(cfg: &McpServerConfig) -> Result<StdioTransport, McpError> {
        let mut cmd = Command::new(&cfg.command);
        cmd.args(&cfg.args)
            .envs(&cfg.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());

        let mut child = cmd
            .spawn()
            .map_err(|e| McpError::Spawn(format!("{}: {e}", cfg.command)))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::Spawn("stdin pipe missing".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::Spawn("stdout pipe missing".into()))?;

        let (tx, replies) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if line.trim().is_empty() {
                    continue;
                }
                if tx.send(line).is_err() {
                    break; // transport dropped
                }
            }
        });

        Ok(StdioTransport {
            io: Mutex::new(Io { stdin, replies }),
            child: Mutex::new(child),
            next: AtomicU64::new(1),
        })
    }
}

impl McpTransport for StdioTransport {
    fn next_id(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    fn notify(&self, line: &str) -> Result<(), McpError> {
        let mut io = lock(&self.io);
        write_line(&mut io.stdin, line)
    }

    fn roundtrip(&self, id: u64, line: &str, signal: &AtomicBool) -> Result<String, McpError> {
        let mut io = lock(&self.io);
        write_line(&mut io.stdin, line)?;

        let deadline = Instant::now() + MCP_TIMEOUT;
        loop {
            if signal.load(Ordering::Relaxed) {
                return Err(McpError::Aborted);
            }
            if Instant::now() >= deadline {
                return Err(McpError::Timeout);
            }
            match io.replies.recv_timeout(POLL_INTERVAL) {
                Ok(reply) => match serde_json::from_str::<Value>(&reply) {
                    Ok(v) => match v.get("id").and_then(Value::as_u64) {
                        Some(got) if got == id => return Ok(reply),
                        // A leftover reply to an earlier request that timed
                        // out: its response arrived after we gave up and is
                        // still queued. Erroring here would desync the channel
                        // permanently — one stale line would poison every
                        // later call — so drop it and keep waiting.
                        Some(got) => {
                            eprintln!("mcp: dropping stale response id {got} (awaiting {id})")
                        }
                        // No id: a server-pushed notification.
                        None => eprintln!(
                            "mcp: dropping server notification: {}",
                            v.get("method")
                                .and_then(Value::as_str)
                                .unwrap_or("<no method>")
                        ),
                    },
                    Err(_) => eprintln!("mcp: dropping non-JSON line from server"),
                },
                Err(RecvTimeoutError::Timeout) => {
                    if Instant::now() >= deadline {
                        return Err(McpError::Timeout);
                    }
                }
                // Reader thread ended: the child closed stdout or exited.
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(McpError::Transport("server closed stdout".into()));
                }
            }
        }
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        // Killing the child ends the reader thread via EOF; a well-behaved
        // server also exits on stdin close, but don't rely on it.
        let mut child = lock(&self.child);
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// A live MCP server plus the tools it advertised at registration.
pub struct Mcp {
    name: String,
    transport: Arc<dyn McpTransport>,
    tools: Vec<ToolMeta>,
}

impl Mcp {
    /// Spawn a server, handshake, and cache its tool list.
    pub fn spawn(cfg: &McpServerConfig, signal: &AtomicBool) -> Result<Mcp, McpError> {
        Mcp::connect(cfg, Arc::new(StdioTransport::spawn(cfg)?), signal)
    }

    /// Handshake over an existing transport. Split out so tests can drive the
    /// protocol without a child process.
    pub fn connect(
        cfg: &McpServerConfig,
        transport: Arc<dyn McpTransport>,
        signal: &AtomicBool,
    ) -> Result<Mcp, McpError> {
        initialize(transport.as_ref(), signal)?;
        let tools = list_tools(transport.as_ref(), &cfg.name, signal)?;
        Ok(Mcp {
            name: cfg.name.clone(),
            transport,
            tools,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// API-facing definitions for the advertised tools.
    pub fn list_tools(&self) -> Vec<ToolDef> {
        self.tools
            .iter()
            .map(|t| ToolDef {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.parameters.clone(),
            })
            .collect()
    }

    /// Invoke one tool by its remote name.
    pub fn call_tool(
        &self,
        remote: &str,
        args: &Value,
        signal: &AtomicBool,
    ) -> Result<String, McpError> {
        call_tool(self.transport.as_ref(), remote, args, signal)
    }

    /// Consume the connection into registrable tools.
    pub fn into_tools(self) -> Vec<Box<dyn Tool>> {
        self.tools
            .into_iter()
            .map(|meta| Box::new(McpTool::new(meta, Arc::clone(&self.transport))) as Box<dyn Tool>)
            .collect()
    }
}

/// Spawn every configured server and collect their tools.
///
/// Default off: an empty `servers` slice spawns nothing and returns nothing, so
/// `builtin_tools()` stays exactly as it was. A server that fails to start,
/// handshake, or list contributes zero tools and is skipped — one broken entry
/// in the user's config must not take down the agent.
pub fn external_tools_from_mcp(
    servers: &[McpServerConfig],
    signal: &AtomicBool,
) -> Vec<Box<dyn Tool>> {
    let mut out = Vec::new();
    for cfg in servers {
        match Mcp::spawn(cfg, signal) {
            Ok(mcp) => out.extend(mcp.into_tools()),
            Err(e) => eprintln!("mcp: skipping server `{}`: {e}", cfg.name),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scripted transport: hands back canned replies in order, recording what
    /// was written. `id` is patched into each reply so `parse_response` sees a
    /// matching response without every test hand-tracking counters.
    struct FakeTransport {
        replies: Mutex<std::collections::VecDeque<String>>,
        sent: Mutex<Vec<String>>,
        notes: Mutex<Vec<String>>,
        next: AtomicU64,
    }

    impl FakeTransport {
        fn new(replies: &[&str]) -> Arc<FakeTransport> {
            Arc::new(FakeTransport {
                replies: Mutex::new(replies.iter().map(|s| s.to_string()).collect()),
                sent: Mutex::new(Vec::new()),
                notes: Mutex::new(Vec::new()),
                next: AtomicU64::new(1),
            })
        }
        fn sent(&self) -> Vec<String> {
            lock(&self.sent).clone()
        }
        fn sent_json(&self, i: usize) -> Value {
            serde_json::from_str(&self.sent()[i]).unwrap()
        }
        fn notes(&self) -> Vec<String> {
            lock(&self.notes).clone()
        }
    }

    impl McpTransport for FakeTransport {
        fn next_id(&self) -> u64 {
            self.next.fetch_add(1, Ordering::Relaxed)
        }
        fn notify(&self, line: &str) -> Result<(), McpError> {
            lock(&self.notes).push(line.to_string());
            Ok(())
        }
        fn roundtrip(&self, id: u64, line: &str, signal: &AtomicBool) -> Result<String, McpError> {
            if signal.load(Ordering::Relaxed) {
                return Err(McpError::Aborted);
            }
            lock(&self.sent).push(line.to_string());
            let reply = lock(&self.replies)
                .pop_front()
                .ok_or_else(|| McpError::Transport("no scripted reply".into()))?;
            let mut v: Value = serde_json::from_str(&reply).unwrap();
            if v.get("id").is_none() {
                v["id"] = json!(id);
            }
            Ok(v.to_string())
        }
    }

    fn sig() -> AtomicBool {
        AtomicBool::new(false)
    }

    fn cfg(name: &str, command: &str) -> McpServerConfig {
        McpServerConfig {
            name: name.to_string(),
            command: command.to_string(),
            args: Vec::new(),
            env: Default::default(),
        }
    }

    const LIST_OK: &str = r#"{"jsonrpc":"2.0","result":{"tools":[
        {"name":"echo","description":"echo it","inputSchema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}},
        {"name":"noargs"}
    ]}}"#;
    const INIT_OK: &str =
        r#"{"jsonrpc":"2.0","result":{"protocolVersion":"2025-06-18","capabilities":{}}}"#;

    #[test]
    fn request_line_is_jsonrpc_2_0() {
        let line = request_line(7, "tools/list", &json!({"cursor": "abc"}));
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["id"], 7);
        assert_eq!(v["method"], "tools/list");
        assert_eq!(v["params"]["cursor"], "abc");
    }

    #[test]
    fn notification_has_no_id() {
        let v: Value =
            serde_json::from_str(&notification_line("notifications/initialized", &json!({})))
                .unwrap();
        assert_eq!(v["jsonrpc"], "2.0");
        assert!(v.get("id").is_none(), "notifications must omit id");
    }

    #[test]
    fn parse_response_unwraps_result() {
        let got = parse_response(r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#, 1).unwrap();
        assert_eq!(got["ok"], true);
    }

    #[test]
    fn parse_response_maps_error_object() {
        let err = parse_response(
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"no such method"}}"#,
            1,
        )
        .unwrap_err();
        match err {
            McpError::Server { code, message } => {
                assert_eq!(code, -32601);
                assert_eq!(message, "no such method");
            }
            other => panic!("expected Server, got {other:?}"),
        }
    }

    #[test]
    fn parse_response_rejects_id_mismatch_and_garbage() {
        let mismatch = parse_response(r#"{"jsonrpc":"2.0","id":9,"result":{}}"#, 1).unwrap_err();
        assert!(matches!(mismatch, McpError::Protocol(_)));
        assert!(matches!(
            parse_response("not json at all", 1).unwrap_err(),
            McpError::Protocol(_)
        ));
    }

    #[test]
    fn initialize_sends_protocol_version_then_acks() {
        let t = FakeTransport::new(&[INIT_OK]);
        initialize(t.as_ref(), &sig()).unwrap();
        let req = t.sent_json(0);
        assert_eq!(req["method"], "initialize");
        assert_eq!(req["params"]["protocolVersion"], PROTOCOL_VERSION);
        let notes = t.notes();
        assert_eq!(notes.len(), 1, "must ack with initialized notification");
        let note: Value = serde_json::from_str(&notes[0]).unwrap();
        assert_eq!(note["method"], "notifications/initialized");
    }

    #[test]
    fn tool_list_maps_to_namespaced_defs() {
        let t = FakeTransport::new(&[LIST_OK]);
        let metas = list_tools(t.as_ref(), "files", &sig()).unwrap();
        assert_eq!(metas.len(), 2);
        assert_eq!(metas[0].name, "mcp__files__echo");
        assert_eq!(metas[0].remote, "echo");
        assert_eq!(metas[0].description, "echo it");
        assert_eq!(metas[0].parameters["required"][0], "text");
        // Missing description/schema degrade to empty + open object schema.
        assert_eq!(metas[1].name, "mcp__files__noargs");
        assert_eq!(metas[1].description, "");
        assert_eq!(metas[1].parameters["type"], "object");
    }

    #[test]
    fn tool_list_without_tools_array_is_protocol_error() {
        let t = FakeTransport::new(&[r#"{"jsonrpc":"2.0","result":{}}"#]);
        assert!(matches!(
            list_tools(t.as_ref(), "s", &sig()).unwrap_err(),
            McpError::Protocol(_)
        ));
    }

    #[test]
    fn tool_list_follows_next_cursor_until_null() {
        // Page 1 has one tool and a non-null cursor; page 2 has another and
        // omits the cursor. Page 1 must be re-issued with the cursor echoed
        // back so paginated servers don't silently drop past page one.
        let t = FakeTransport::new(&[
            r#"{"jsonrpc":"2.0","result":{"tools":[{"name":"a"}],"nextCursor":"c1"}}"#,
            r#"{"jsonrpc":"2.0","result":{"tools":[{"name":"b"}],"nextCursor":null}}"#,
        ]);
        let metas = list_tools(t.as_ref(), "s", &sig()).unwrap();
        assert_eq!(metas.len(), 2);
        assert_eq!(metas[0].remote, "a");
        assert_eq!(metas[1].remote, "b");
        let page2 = t.sent_json(1);
        assert_eq!(page2["params"]["cursor"], "c1");
    }

    #[test]
    fn call_tool_sends_name_and_args_and_joins_text() {
        let t = FakeTransport::new(&[
            r#"{"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"hi"},{"type":"text","text":"there"}]}}"#,
        ]);
        let out = call_tool(t.as_ref(), "echo", &json!({"text": "hi"}), &sig()).unwrap();
        assert_eq!(out, "hi\nthere");
        let req = t.sent_json(0);
        assert_eq!(req["method"], "tools/call");
        assert_eq!(req["params"]["name"], "echo");
        assert_eq!(req["params"]["arguments"]["text"], "hi");
    }

    #[test]
    fn call_tool_normalizes_non_object_args() {
        let t = FakeTransport::new(&[r#"{"jsonrpc":"2.0","result":{"content":[]}}"#]);
        call_tool(t.as_ref(), "noargs", &Value::Null, &sig()).unwrap();
        assert!(t.sent_json(0)["params"]["arguments"].is_object());
    }

    #[test]
    fn call_tool_maps_is_error_to_tool_error() {
        let t = FakeTransport::new(&[
            r#"{"jsonrpc":"2.0","result":{"isError":true,"content":[{"type":"text","text":"boom"}]}}"#,
        ]);
        match call_tool(t.as_ref(), "echo", &json!({}), &sig()).unwrap_err() {
            McpError::Tool(m) => assert_eq!(m, "boom"),
            other => panic!("expected Tool, got {other:?}"),
        }
    }

    #[test]
    fn mcp_tool_delegates_execute_to_tools_call() {
        let t = FakeTransport::new(&[
            INIT_OK,
            LIST_OK,
            r#"{"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"pong"}]}}"#,
        ]);
        let mcp = Mcp::connect(&cfg("files", "unused"), t.clone(), &sig()).unwrap();
        assert_eq!(
            mcp.list_tools()
                .iter()
                .map(|d| d.name.clone())
                .collect::<Vec<_>>(),
            ["mcp__files__echo", "mcp__files__noargs"]
        );

        let registered = mcp.into_tools();
        assert_eq!(registered.len(), 2);
        let echo = &registered[0];
        assert_eq!(echo.name(), "mcp__files__echo");
        assert_eq!(echo.description(), "echo it");
        assert_eq!(
            echo.execute(&json!({"text": "ping"}), &sig()).unwrap(),
            "pong"
        );
        // The remote (un-namespaced) name goes over the wire.
        assert_eq!(t.sent_json(2)["params"]["name"], "echo");
    }

    #[test]
    fn signal_poll_aborts_before_sending() {
        let t = FakeTransport::new(&[INIT_OK]);
        let signal = AtomicBool::new(true);
        assert!(matches!(
            request(t.as_ref(), "tools/list", &json!({}), &signal).unwrap_err(),
            McpError::Aborted
        ));
        assert!(t.sent().is_empty(), "aborted request must not be sent");
    }

    #[test]
    fn abort_surfaces_through_tool_execute() {
        let t = FakeTransport::new(&[INIT_OK, LIST_OK]);
        let tools = Mcp::connect(&cfg("files", "unused"), t, &sig())
            .unwrap()
            .into_tools();
        let err = tools[0]
            .execute(&json!({"text": "x"}), &AtomicBool::new(true))
            .unwrap_err();
        assert_eq!(err.to_string(), "aborted");
    }

    #[test]
    fn spawn_failure_yields_no_tools_and_does_not_panic() {
        let servers = [cfg("broken", "/nonexistent/definitely/not/here/mcp-server")];
        assert!(external_tools_from_mcp(&servers, &sig()).is_empty());
    }

    #[test]
    fn no_servers_configured_spawns_nothing() {
        assert!(external_tools_from_mcp(&[], &sig()).is_empty());
    }

    /// End-to-end over a real child process: `cat` echoes back whatever we
    /// write, so a scripted reply file is unnecessary — but `cat` can't answer
    /// JSON-RPC, so this only proves spawn + timeout wiring, not a handshake.
    #[test]
    fn stdio_transport_spawns_and_reports_closed_stdout() {
        let t = StdioTransport::spawn(&cfg("t", "true")).expect("`true` should spawn");
        // `true` exits immediately: stdout closes, reader thread ends.
        match t.roundtrip(1, &request_line(1, "initialize", &json!({})), &sig()) {
            Err(McpError::Transport(_)) | Err(McpError::Timeout) => {}
            other => panic!("expected transport/timeout error, got {other:?}"),
        }
    }

    #[test]
    fn stdio_transport_roundtrips_against_a_real_child() {
        // A one-line `sh` server: read a request, reply with a fixed result.
        let mut c = cfg("shell", "sh");
        c.args = vec![
            "-c".into(),
            r#"read -r line; printf '{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"ok"}]}}\n'"#
                .into(),
        ];
        let t = StdioTransport::spawn(&c).unwrap();
        let out = call_tool(&t, "echo", &json!({}), &sig()).unwrap();
        assert_eq!(out, "ok");
    }

    /// A reply left over from a timed-out earlier request must be skipped, not
    /// turned into a protocol error that desyncs the channel for good.
    #[test]
    fn stdio_transport_skips_stale_reply_and_notification() {
        let mut c = cfg("shell", "sh");
        c.args = vec![
            "-c".into(),
            // Queued ahead of the real answer: a response to request id 1
            // (which we pretend timed out) and an id-less notification.
            r#"read -r line
printf '{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"stale"}]}}\n'
printf '{"jsonrpc":"2.0","method":"notifications/message","params":{}}\n'
printf '{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"fresh"}]}}\n'"#
                .into(),
        ];
        let t = StdioTransport::spawn(&c).unwrap();
        let line = t
            .roundtrip(2, &request_line(2, "tools/call", &json!({})), &sig())
            .expect("stale reply must not fail the live request");
        assert_eq!(
            parse_response(&line, 2).unwrap()["content"][0]["text"],
            "fresh"
        );
    }
}
