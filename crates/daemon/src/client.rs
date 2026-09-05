//! Typed client for the omenic daemon.
//!
//! The client is a thin wrapper over the daemon's newline-delimited JSON
//! protocol (see [`crate::protocol`]). It connects to the Unix-domain
//! socket via [`config::Config::daemon_socket_path`] (honoring
//! `OMENIC_DAEMON_SOCKET`), sends one request per connection, reads the
//! matching response, and returns the typed result.
//!
//! Reconnect is a deliberate no-op: a fresh connection is opened on every
//! call, which keeps the API boring and survives server restarts without
//! extra bookkeeping. Tests can use [`DaemonClient::connect_to`] to point
//! at a fixture socket path.
//!
//! ponytail: the client is sync because every caller is sync and the
//! daemon itself answers on a dedicated thread. Re-introduce async only if
//! a caller actually needs to overlap several requests.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::state::RunRecord;
use session::{SessionMessage, SessionRole, SessionSummary};

use crate::DaemonError;
use crate::protocol::{Command, Request, Response};

/// Errors returned from the daemon client.
#[derive(Debug)]
pub enum ClientError {
    /// Could not reach the daemon socket (no daemon running, stale lock, etc.).
    Connect(std::io::Error),
    /// Server reply was malformed JSON or did not match the expected shape.
    Protocol(String),
    /// Server replied with a structured error.
    Server { code: String, message: String },
    /// Failed to serialize a request.
    Encode(serde_json::Error),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Connect(e) => write!(f, "daemon connection error: {e}"),
            ClientError::Protocol(s) => write!(f, "daemon protocol error: {s}"),
            ClientError::Server { code, message } => write!(f, "daemon error `{code}`: {message}"),
            ClientError::Encode(e) => write!(f, "daemon request encode error: {e}"),
        }
    }
}

impl std::error::Error for ClientError {}

impl From<ClientError> for DaemonError {
    fn from(e: ClientError) -> Self {
        match e {
            ClientError::Connect(io) => DaemonError::Io(io),
            other => DaemonError::Protocol(other.to_string()),
        }
    }
}

/// Thin client. Cloning is cheap (just the socket path).
#[derive(Debug, Clone)]
pub struct DaemonClient {
    socket: PathBuf,
}

impl DaemonClient {
    /// Build a client rooted at the daemon socket resolved by [`config::Config`].
    pub fn from_config(cfg: &config::Config) -> Result<Self, config::ConfigError> {
        Ok(DaemonClient {
            socket: cfg.daemon_socket_path()?,
        })
    }

    /// Build a client pointing at an explicit socket path. Tests use this.
    pub fn connect_to(socket: impl Into<PathBuf>) -> Self {
        DaemonClient {
            socket: socket.into(),
        }
    }

    /// Socket path the client will connect to.
    pub fn socket_path(&self) -> &Path {
        &self.socket
    }

    /// Round-trip a single request and return the typed response. Every
    /// public command funnels through here; tests rely on the `success`
    /// field for assertions.
    pub fn call_raw(&self, command: Command, params: Value) -> Result<Response, ClientError> {
        let req = Request::new(command).with_params(params);
        let mut conn = connect(&self.socket).map_err(ClientError::Connect)?;
        write_frame(&mut conn, &req).map_err(ClientError::Connect)?;
        read_frame(&mut conn).map_err(ClientError::Connect)
    }

    /// Like [`Self::call_raw`], but unwraps a successful `data` payload into
    /// `T`. Returns a structured error when the daemon replied with
    /// `success: false` or the payload could not be deserialized.
    pub fn call<T: DeserializeOwned>(
        &self,
        command: Command,
        params: Value,
    ) -> Result<T, ClientError> {
        let resp = self.call_raw(command, params)?;
        if !resp.success {
            return Err(match resp.error {
                Some(e) => ClientError::Server {
                    code: e.code,
                    message: e.message,
                },
                None => ClientError::Protocol("response missing error envelope".into()),
            });
        }
        let data = resp.data.unwrap_or(Value::Null);
        serde_json::from_value(data).map_err(|e| ClientError::Protocol(format!("decode: {e}")))
    }

    /// `daemon.ping` round-trip. Always returns `Ok(true)` when the
    /// daemon is up — the wire format is `{ "pong": true }` and we
    /// unwrap it for the caller.
    pub fn ping(&self) -> Result<bool, ClientError> {
        #[derive(serde::Deserialize)]
        struct Pong {
            pong: bool,
        }
        let v: Pong = self.call(Command::Ping, Value::Null)?;
        Ok(v.pong)
    }

    /// `daemon.info` → `{ pid, started_at_ms, uptime_ms, worker_pid }`.
    pub fn info(&self) -> Result<DaemonInfo, ClientError> {
        self.call(Command::Info, Value::Null)
    }

    /// `session.create` → created session summary.
    pub fn session_create(
        &self,
        session_id: &str,
        title: &str,
    ) -> Result<SessionSummary, ClientError> {
        self.call(
            Command::SessionCreate,
            json!({ "session_id": session_id, "title": title }),
        )
    }

    /// `session.list` → up to `limit` matching summaries.
    pub fn session_list(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<SessionSummary>, ClientError> {
        self.call(
            Command::SessionList,
            json!({ "query": query, "limit": limit }),
        )
    }

    /// `session.get` → one summary, or `None` if no such session.
    pub fn session_get(&self, session_id: &str) -> Result<Option<SessionSummary>, ClientError> {
        let v: Value = self.call(Command::SessionGet, json!({ "session_id": session_id }))?;
        if v.is_null() {
            Ok(None)
        } else {
            serde_json::from_value(v)
                .map_err(|e| ClientError::Protocol(format!("decode summary: {e}")))
        }
    }

    /// `session.delete` → `true` if a row was removed.
    pub fn session_delete(&self, session_id: &str) -> Result<bool, ClientError> {
        let v: DeleteOutcome =
            self.call(Command::SessionDelete, json!({ "session_id": session_id }))?;
        Ok(v.deleted)
    }

    /// `session.append` → assigned seq + timestamp.
    pub fn session_append(
        &self,
        session_id: &str,
        role: SessionRole,
        text: &str,
    ) -> Result<AppendOutcome, ClientError> {
        self.call(
            Command::SessionAppend,
            json!({
                "session_id": session_id,
                "role": role.as_str(),
                "text": text,
            }),
        )
    }

    /// `session.load_messages` → up to `limit` messages.
    pub fn session_load_messages(
        &self,
        session_id: &str,
        limit: u32,
    ) -> Result<Vec<SessionMessage>, ClientError> {
        self.call(
            Command::SessionLoadMessages,
            json!({ "session_id": session_id, "limit": limit }),
        )
    }

    /// `session.search` → up to `limit` messages whose text matches `query`.
    /// When `scope` is `Some(id)`, the search is restricted to that session.
    pub fn session_search(
        &self,
        query: &str,
        scope: Option<&str>,
        limit: u32,
    ) -> Result<Vec<SessionMessage>, ClientError> {
        let params = match scope {
            Some(id) => json!({ "query": query, "scope": { "id": id }, "limit": limit }),
            None => json!({ "query": query, "limit": limit }),
        };
        self.call(Command::SessionSearch, params)
    }

    /// Read run records newer than `cursor` and return the next cursor.
    pub fn read_from_cursor(
        &self,
        cursor: i64,
    ) -> Result<(Vec<crate::state::RunRecord>, i64), ClientError> {
        #[derive(serde::Deserialize)]
        struct Replay {
            runs: Vec<crate::state::RunRecord>,
            cursor: i64,
        }
        let replay: Replay =
            self.call(Command::SessionReadFromCursor, json!({ "cursor": cursor }))?;
        Ok((replay.runs, replay.cursor))
    }

    /// Agent-facing entry point. Routes a generic `session_query` payload
    /// (kind + args) to the daemon and returns the raw JSON. Used by the
    /// `session_query` external tool — the agent-facing name shows up in
    /// the ToolDef, but the daemon itself is the data plane.
    pub fn session_query(&self, args: &Value) -> Result<Value, ClientError> {
        let kind = args
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| ClientError::Protocol("missing `kind` field".into()))?;
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(session::MAX_LIMIT);
        match kind {
            "list" => {
                let q = args
                    .get("query")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ClientError::Protocol("`query` is required".into()))?;
                let rows = self.session_list(q, limit)?;
                Ok(serde_json::to_value(rows).map_err(ClientError::Encode)?)
            }
            "get" => {
                let id = args
                    .get("session_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ClientError::Protocol("`session_id` is required".into()))?;
                let row = self.session_get(id)?;
                Ok(serde_json::to_value(row).map_err(ClientError::Encode)?)
            }
            "search" => {
                let q = args
                    .get("query")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ClientError::Protocol("`query` is required".into()))?;
                let scope = args.get("scope_id").and_then(Value::as_str);
                let rows = self.session_search(q, scope, limit)?;
                Ok(serde_json::to_value(rows).map_err(ClientError::Encode)?)
            }
            "delete" => {
                let id = args
                    .get("session_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ClientError::Protocol("`session_id` is required".into()))?;
                let deleted = self.session_delete(id)?;
                Ok(json!({ "deleted": deleted }))
            }
            other => Err(ClientError::Protocol(format!(
                "unknown session_query kind `{other}`"
            ))),
        }
    }

    /// `run.list` → up to `limit` runs (empty list allowed).
    pub fn run_list(&self, limit: u32) -> Result<Vec<RunRecord>, ClientError> {
        self.call(Command::RunList, json!({ "limit": limit }))
    }
}

/// Payload returned by `daemon.info`.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct DaemonInfo {
    pub pid: u32,
    #[serde(rename = "started_at_ms")]
    pub started_at_ms: i64,
    #[serde(rename = "uptime_ms")]
    pub uptime_ms: i64,
    #[serde(rename = "worker_pid")]
    pub worker_pid: u32,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
struct DeleteOutcome {
    deleted: bool,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct AppendOutcome {
    pub seq: i64,
    #[serde(rename = "created_at_ms")]
    pub created_at_ms: i64,
}

// ---------------- transport ----------------
//
// ponytail: the Unix variant returns a concrete `UnixStream`; the
// non-Unix stub is unreachable on the targets we ship today, but kept so
// the file type-checks on Windows. `call_raw` threads the concrete type
// through, so the unix path stays one `connect → write → read` sequence
// without an extra allocation.

#[cfg(target_family = "unix")]
type Stream = std::os::unix::net::UnixStream;

#[cfg(not(target_family = "unix"))]
type Stream = std::io::Empty;

#[cfg(target_family = "unix")]
fn connect(path: &Path) -> std::io::Result<Stream> {
    std::os::unix::net::UnixStream::connect(path)
}

#[cfg(not(target_family = "unix"))]
fn connect(_path: &Path) -> std::io::Result<Stream> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "daemon client requires Unix-domain sockets; Windows named-pipe support is a TODO",
    ))
}

fn write_frame(conn: &mut Stream, req: &Request) -> std::io::Result<()> {
    let payload = serde_json::to_string(req).map_err(std::io::Error::other)?;
    conn.write_all(payload.as_bytes())?;
    conn.write_all(b"\n")
}

fn read_frame(conn: &mut Stream) -> std::io::Result<Response> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = conn.read(&mut byte)?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "daemon closed the connection before sending a response",
            ));
        }
        if byte[0] == b'\n' {
            break;
        }
        buf.push(byte[0]);
    }
    serde_json::from_slice(&buf).map_err(std::io::Error::other)
}
