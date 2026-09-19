//! MCP (Model Context Protocol) client: JSON-RPC 2.0 over stdio or HTTP.
//!
//! Default off — nothing is spawned unless `[[mcp.servers]]` is configured.
//! An MCP server is a child process; its `tools/list` entries map onto the
//! built-in [`tools::Tool`] trait so the agent loop can't tell them apart from
//! native tools.
//!
//! Layering: JSON-RPC framing is free functions over an [`McpTransport`], so
//! the protocol is testable without spawning anything. [`StdioTransport`] is
//! the process implementation; [`HttpTransport`] posts to a running server;
//! [`McpReconnect`] wraps either with exponential-backoff retries.

pub mod http;
pub mod reconnect;
pub mod tool;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use adaptor::ToolDef;
use config::McpServerConfig;
use serde_json::{Value, json};
use tools::{Tool, ToolError};

pub use http::HttpTransport;
pub use reconnect::{McpReconnect, ReconnectPolicy};
pub use tool::{McpTool, ToolMeta};

/// MCP basic spec revision this client implements.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Per-request budget and abort poll interval. Matches the built-in tools'
/// subprocess timeout so a hung server can't wedge the agent loop.
pub const MCP_TIMEOUT: Duration = Duration::from_secs(30);
pub const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Bound `tools/list` pagination: a server that keeps handing back a fresh
/// cursor must not spin the agent forever.
const MAX_TOOL_LIST_PAGES: usize = 64;

// SIGKILL a whole process group. Declared here rather than pulling in `libc`,
// which mcp does not depend on directly; the symbol comes from libc via std.
unsafe extern "C" {
    fn killpg(pgrp: i32, sig: i32) -> i32;
}

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

impl McpError {
    /// Prefix the message with the config entry the error came from.
    ///
    /// The *variant* is preserved on purpose: callers match on it to tell a
    /// spawn failure from a handshake failure, so re-wrapping in `Protocol`
    /// just to attach a name would erase that distinction.
    fn named(self, server: &str) -> McpError {
        match self {
            McpError::Spawn(m) => McpError::Spawn(format!("server `{server}`: {m}")),
            McpError::Transport(m) => McpError::Transport(format!("server `{server}`: {m}")),
            McpError::Protocol(m) => McpError::Protocol(format!("server `{server}`: {m}")),
            McpError::Server { code, message } => McpError::Server {
                code,
                message: format!("server `{server}`: {message}"),
            },
            McpError::Tool(m) => McpError::Tool(format!("server `{server}`: {m}")),
            // Unit variants carry no message to prefix; adding a "which
            // server" field to them is a public-API change of its own.
            McpError::Timeout => McpError::Timeout,
            McpError::Aborted => McpError::Aborted,
        }
    }
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
/// Follows `nextCursor` until it is absent, empty, repeated, or reaches the
/// bounded page count, so paginated servers don't silently drop everything
/// past page one or spin forever on malformed cursors.
pub fn list_tools(
    transport: &dyn McpTransport,
    server: &str,
    signal: &AtomicBool,
) -> Result<Vec<ToolMeta>, McpError> {
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    let mut attempts = 0usize;
    loop {
        let params = match &cursor {
            Some(c) => json!({ "cursor": c }),
            None => json!({}),
        };
        let result = request(transport, "tools/list", &params, signal)?;
        attempts += 1;
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
            // Namespaced name goes on the wire, where OpenAI enforces
            // `^[a-zA-Z0-9_-]{1,64}$`; a server is free to advertise anything.
            let name = sanitize_tool_name(&format!("mcp__{server}__{remote}"));
            if name.is_empty() {
                eprintln!("mcp: skipping tool `{remote}`: name empty after sanitizing");
                continue;
            }
            out.push(ToolMeta {
                name,
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
        let next = match result.get("nextCursor") {
            Some(Value::Null) | None => break,
            Some(v) => match v.as_str() {
                Some("") | None => break,
                Some(s) => s,
            },
        };
        // A repeated cursor or a runaway page count means the server is broken.
        if cursor.as_deref() == Some(next) || attempts >= MAX_TOOL_LIST_PAGES {
            eprintln!("mcp: stopping tools/list pagination after {attempts} page(s)");
            break;
        }
        cursor = Some(next.to_string());
    }
    Ok(out)
}

/// Coerce a tool name into `[a-zA-Z0-9_-]`, capped at 64 characters (all ASCII
/// after substitution, so also 64 bytes).
fn sanitize_tool_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect()
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
        let command = cfg
            .command
            .clone()
            .filter(|c| !c.trim().is_empty())
            .ok_or_else(|| McpError::Spawn("no command for stdio transport".into()))?;
        let mut cmd = Command::new(&command);
        cmd.args(&cfg.args)
            .envs(&cfg.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            // Own process group: Drop can then reap the server's children too.
            .process_group(0);

        // Optional per-server working directory: the OS resolves `command`
        // and relative `args` paths against it. Empty/whitespace means
        // inherit, same as an absent field. A bad path is not pre-checked —
        // `spawn` surfaces the OS error, and the message below names the cwd
        // so a missing directory is distinguishable from a missing program.
        let cwd = cfg.cwd.as_deref().map(str::trim).filter(|d| !d.is_empty());
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        let mut child = cmd.spawn().map_err(|e| match cwd {
            Some(dir) => McpError::Spawn(format!("cwd {dir}: {command}: {e}")),
            None => McpError::Spawn(format!("{command}: {e}")),
        })?;

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
        // server also exits on stdin close, but don't rely on it. The server
        // gets its own process group at spawn, so signal the group to take any
        // helper processes it forked with it.
        let mut child = lock(&self.child);
        if unsafe { killpg(child.id() as i32, 9) } != 0 {
            eprintln!(
                "mcp: failed to kill process group: {}",
                std::io::Error::last_os_error()
            );
            let _ = child.kill();
        }
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

    /// Connect over streamable-HTTP with an optional reconnect policy.
    ///
    /// The server must already be running — this transport spawns nothing.
    /// `cfg.url` is required; `cfg.command` is ignored.
    pub fn connect_http(cfg: &McpServerConfig, signal: &AtomicBool) -> Result<Mcp, McpError> {
        let url = cfg
            .url
            .clone()
            .ok_or_else(|| McpError::Spawn("no url for http transport".into()))?;
        let timeout_ms = cfg
            .tool_call_timeout_ms
            .unwrap_or_else(|| MCP_TIMEOUT.as_millis() as u64);
        let transport: Arc<dyn McpTransport> = Arc::new(HttpTransport::new(url, timeout_ms));
        let defaults = ReconnectPolicy::default();
        let policy = match &cfg.reconnect {
            Some(r) => ReconnectPolicy {
                initial_delay_ms: r.initial_delay_ms.unwrap_or(defaults.initial_delay_ms),
                max_delay_ms: r.max_delay_ms.unwrap_or(defaults.max_delay_ms),
                max_attempts: r.max_attempts.unwrap_or(defaults.max_attempts),
            },
            None => defaults,
        };
        let wrapped = McpReconnect::new(transport, policy);
        Self::connect(cfg, Arc::new(wrapped), signal)
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
/// in the user's config must not take down the agent. A server that sets
/// `fail_on_startup_error = true` opts out: its first failure is returned as
/// `Err` and aborts the whole bring-up instead of being silently skipped.
pub fn external_tools_from_mcp(
    servers: &[McpServerConfig],
    signal: &AtomicBool,
) -> Result<Vec<Box<dyn Tool>>, McpError> {
    let mut out = Vec::new();
    for cfg in servers {
        // A configured `url` means streamable-HTTP; otherwise spawn a stdio child.
        let res: Result<Mcp, McpError> = if cfg.url.is_some() {
            Mcp::connect_http(cfg, signal)
        } else {
            Mcp::spawn(cfg, signal)
        };
        match res {
            Ok(mcp) => out.extend(mcp.into_tools()),
            Err(e) => {
                // Name the config entry that failed: callers surface this
                // error verbatim (e.g. daemon bring-up), and a bare command
                // string does not tell the user which server row to fix.
                // `named` keeps the variant — `startup_policy` matches on
                // `Spawn` vs `Transport` to tell "cannot start the child"
                // from "started but not an MCP server".
                let e = e.named(&cfg.name);
                if cfg.fail_on_startup_error == Some(true) {
                    return Err(e);
                }
                eprintln!("mcp: skipping server `{}`: {e}", cfg.name);
            }
        }
    }
    Ok(out)
}
