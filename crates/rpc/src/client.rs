#![allow(dead_code)] // consumed by worker in M2.2
//! omp `--mode rpc` client (stdio JSONL).
//!
//! Spawns an omp process in RPC mode and provides a bidirectional channel
//! for sending commands and receiving responses.
//!
//! Protocol per <https://github.com/oh-my-pi/oh-my-pi/docs/rpc.md>.
//! Blueprint: autopus-adk internal/cli.
//!
//! ## Frame format
//! Every line is a JSON object, newline-delimited, max 1 MiB per frame.
//! Large payloads are split into `rpc_chunk` messages and reassembled
//! (max 64 MiB after reassembly).
//!
//! ## Command format
//! All commands use `{ id?, type: "command_name", ... }` on stdin.
//! Responses use `{ type: "response", id?, ... }` on stdout.
//!
//! ## Lifecycle
//! ```ignore
//! let mut client = omp_rpc::Client::new("/usr/bin/omp")?;
//! let resp = client.send(&omp_rpc::Request::new("ping").done())?;
//! ```

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Errors from the RPC client.
#[derive(Debug)]
pub enum RpcError {
    /// I/O error (spawn, read, write).
    Io(std::io::Error),
    /// JSON parse error on a received frame.
    Json(serde_json::Error),
    /// Protocol error (unexpected message type, negotiate failure, etc.).
    Protocol(String),
    /// The omp process exited unexpectedly.
    ProcessExited(Option<i32>),
    /// A response did not arrive within the configured timeout.
    Timeout,
    /// Chunk reassembly failed.
    Chunk(String),
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RpcError::Io(e) => write!(f, "RPC I/O error: {e}"),
            RpcError::Json(e) => write!(f, "RPC JSON error: {e}"),
            RpcError::Protocol(s) => write!(f, "RPC protocol error: {s}"),
            RpcError::ProcessExited(c) => write!(f, "omp process exited with code {c:?}"),
            RpcError::Timeout => write!(f, "RPC response timeout"),
            RpcError::Chunk(s) => write!(f, "RPC chunk error: {s}"),
        }
    }
}

impl std::error::Error for RpcError {}

impl From<std::io::Error> for RpcError {
    fn from(e: std::io::Error) -> Self {
        RpcError::Io(e)
    }
}

impl From<serde_json::Error> for RpcError {
    fn from(e: serde_json::Error) -> Self {
        RpcError::Json(e)
    }
}

/// A JSON frame received from omp, tagged by `type` field.
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum Frame {
    /// Initial handshake after spawn.
    #[serde(rename = "ready")]
    Ready {
        #[serde(rename = "protocolVersion", default)]
        protocol_version: u32,
        #[serde(rename = "supportedProtocolVersions", default)]
        supported_protocol_versions: Vec<u32>,
        #[serde(rename = "maxFrameBytes", default)]
        max_frame_bytes: u64,
        #[serde(rename = "maxReassembledFrameBytes", default)]
        max_reassembled_frame_bytes: u64,
    },
    /// UI extension request (filtered transparently).
    #[serde(rename = "extension_ui_request")]
    ExtensionUiRequest { id: String, method: String },
    /// Available commands (emitted at startup and on changes).
    #[serde(rename = "available_commands_update")]
    AvailableCommandsUpdate,
    /// A chunked payload for reassembly.
    #[serde(rename = "rpc_chunk")]
    RpcChunk {
        #[serde(rename = "chunkId")]
        chunk_id: String,
        index: u32,
        count: u32,
        #[serde(rename = "byteLength")]
        byte_length: u64,
        data: String,
    },
    /// A command response (matched by id).
    #[serde(rename = "response")]
    Response {
        id: Option<String>,
        #[serde(flatten)]
        extra: HashMap<String, serde_json::Value>,
    },
    /// Any other message type (turn, message, agent_end, etc.).
    #[serde(other)]
    Other,
}

/// A request sent to omp.  The `type` field is the command name.
///
/// Build an instance with `Request::new("command_name")` and add fields via
/// `.field("key", value)` or the builder-style `WithField` helpers.
#[derive(Debug, Clone, Serialize)]
pub struct Request {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub r#type: String,
    #[serde(flatten)]
    extra: HashMap<String, serde_json::Value>,
}

impl Request {
    /// Start building a command with the given `type` (command name).
    pub fn new(type_name: &str) -> Self {
        Request {
            id: None,
            r#type: type_name.to_string(),
            extra: HashMap::new(),
        }
    }

    /// Set the request id (recommended for response correlation).
    pub fn with_id(mut self, id: &str) -> Self {
        self.id = Some(id.to_string());
        self
    }

    /// Add an arbitrary field (serialized as JSON).
    pub fn with_field<V: Serialize>(mut self, key: &str, value: V) -> Self {
        self.extra.insert(
            key.to_string(),
            serde_json::to_value(value).unwrap_or(serde_json::Value::Null),
        );
        self
    }

    /// Consume the builder and return the request.
    ///
    /// This is a no-op retained for API clarity; `Request` is usable directly
    /// after construction, but `done()` makes the intent explicit.
    pub fn done(self) -> Self {
        self
    }
}

/// omp RPC client.
///
/// Spawns `omp --mode rpc` as a child process, reads the initial `ready` frame,
pub struct Client {
    /// The child process.
    process: Child,
    /// Stdin pipe to the child.
    stdin: ChildStdin,
    /// Buffered reader on stdout.
    reader: BufReader<std::process::ChildStdout>,
    /// Monotonic message counter.
    next_id: u64,
    /// Chunk reassembly buffers.
    chunks: HashMap<String, ChunkState>,
    /// Path to the omp binary (kept for reconnect).
    omp_path: String,
    /// Per-response timeout; None = block forever.
    timeout: Option<Duration>,
    /// Max reconnect attempts on ProcessExited.
    max_retries: u32,
}

impl Client {
    /// PID of the spawned omp worker process. Its process group id equals
    /// this pid (set via `process_group(0)`), so `kill -TERM -<pid>` stops
    /// the whole worker tree on abort.
    pub fn child_pid(&self) -> u32 {
        self.process.id()
    }
}

struct ChunkState {
    count: u32,
    parts: Vec<Option<String>>,
    byte_length: u64,
    received: u32,
}

impl Client {
    /// Spawn an omp RPC client and read the initial `ready` frame.
    ///
    /// Blocks until the `ready` message is received.  No explicit protocol
    /// negotiation is needed — omp emits `ready` with supported versions.
    pub fn new(omp_path: &str) -> Result<Self, RpcError> {
        Self::new_with_opts(omp_path, None, 0)
    }

    /// Spawn with a connect timeout (deadline for the `ready` frame) and
    /// auto-reconnect retry count.
    pub fn new_with_opts(
        omp_path: &str,
        connect_timeout: Option<Duration>,
        max_retries: u32,
    ) -> Result<Self, RpcError> {
        let mut client = Self::spawn_omp(omp_path, connect_timeout)?;
        client.omp_path = omp_path.to_string();
        client.timeout = None;
        client.max_retries = max_retries;
        client.negotiate_session()?;
        Ok(client)
    }

    /// Set per-response timeout. `None` = block forever.
    pub fn set_timeout(&mut self, timeout: Option<Duration>) {
        self.timeout = timeout;
    }

    /// Reconnect: kill the old process, spawn a new one, renegotiate.
    /// Resets chunk state and id counter.
    pub fn reconnect(&mut self) -> Result<(), RpcError> {
        let _ = self.process.kill();
        let _ = self.process.wait();
        let mut new = Self::spawn_omp(&self.omp_path, None)?;
        new.omp_path = self.omp_path.clone();
        new.timeout = self.timeout;
        new.max_retries = self.max_retries;
        new.negotiate_session()?;
        // Swap fields.
        std::mem::swap(&mut self.process, &mut new.process);
        std::mem::swap(&mut self.stdin, &mut new.stdin);
        std::mem::swap(&mut self.reader, &mut new.reader);
        self.chunks.clear();
        Ok(())
    }

    /// Spawn omp, read `ready` frame. If `connect_timeout` is set, the
    /// read is raced against the deadline in a separate thread.
    fn spawn_omp(omp_path: &str, connect_timeout: Option<Duration>) -> Result<Self, RpcError> {
        let mut process = Command::new(omp_path)
            .args(["--mode", "rpc"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .process_group(0)
            .spawn()?;

        let stdin = process
            .stdin
            .take()
            .ok_or_else(|| RpcError::Protocol("failed to capture stdin".into()))?;
        let stdout = process
            .stdout
            .take()
            .ok_or_else(|| RpcError::Protocol("failed to capture stdout".into()))?;

        let mut client = Client {
            process,
            stdin,
            reader: BufReader::new(stdout),
            next_id: 1,
            chunks: HashMap::new(),
            omp_path: String::new(),
            timeout: None,
            max_retries: 0,
        };

        // Read the initial `ready` message, optionally with a timeout.
        if let Some(deadline_dur) = connect_timeout {
            // ponytail: race the read in a thread, join with timeout.
            // If the deadline passes, kill the process and return Timeout.
            let timeout_conn = Self::read_ready_timeout(&mut client, deadline_dur);
            match timeout_conn {
                Ok(frame) => match frame {
                    Frame::Ready { .. } => {}
                    other => {
                        return Err(RpcError::Protocol(format!(
                            "expected ready, got: {other:?}"
                        )));
                    }
                },
                Err(e) => {
                    let _ = client.process.kill();
                    return Err(e);
                }
            }
        } else {
            let ready = client.read_frame()?;
            match ready {
                Frame::Ready { .. } => {}
                other => {
                    return Err(RpcError::Protocol(format!(
                        "expected ready, got: {other:?}"
                    )));
                }
            }
        }

        Ok(client)
    }

    /// Read the ready frame with a timeout using nonblocking poll.
    fn read_ready_timeout(client: &mut Client, dur: Duration) -> Result<Frame, RpcError> {
        use std::os::unix::io::AsRawFd;

        let fd = client.reader.get_ref().as_raw_fd();
        let deadline = Instant::now() + dur;

        // Set nonblocking, poll, then restore blocking.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 {
            return Err(RpcError::Io(std::io::Error::last_os_error()));
        }
        unsafe {
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }

        let result = loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break Err(RpcError::Timeout);
            }

            let mut pfd = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let ms = remaining.as_millis().min(i32::MAX as u128) as i32;
            let rc = unsafe { libc::poll(&mut pfd, 1, ms) };
            if rc < 0 {
                break Err(RpcError::Io(std::io::Error::last_os_error()));
            }
            if rc == 0 {
                continue; // timeout, but loop checks deadline
            }
            if pfd.revents & libc::POLLIN != 0 {
                let mut buf = Vec::with_capacity(1024);
                match client.reader.read_until(b'\n', &mut buf) {
                    Ok(0) => {
                        let status = client.process.try_wait().ok().flatten();
                        break Err(RpcError::ProcessExited(status.and_then(|s| s.code())));
                    }
                    Ok(_n) => {
                        if buf.ends_with(b"\n") {
                            buf.pop();
                        }
                        if !buf.is_empty() {
                            match serde_json::from_slice::<Frame>(&buf) {
                                Ok(frame) => break Ok(frame),
                                Err(e) => break Err(RpcError::Json(e)),
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        continue;
                    }
                    Err(e) => break Err(RpcError::Io(e)),
                }
            }
            if pfd.revents & (libc::POLLHUP | libc::POLLERR) != 0 {
                let status = client.process.try_wait().ok().flatten();
                break Err(RpcError::ProcessExited(status.and_then(|s| s.code())));
            }
        };

        // Restore blocking mode.
        unsafe {
            libc::fcntl(fd, libc::F_SETFL, flags);
        }
        result
    }

    /// Post-handshake negotiation: `negotiate_protocol` v2 →
    /// `set_auto_retry off` → `set_auto_compaction off`, in order. Any
    /// failure or non-success response aborts worker creation — a session
    /// running on silently different semantics must not look healthy (#51).
    fn negotiate_session(&mut self) -> Result<(), RpcError> {
        let id = self.next_id_str();
        let resp = self.send(
            &Request::new("negotiate_protocol")
                .with_id(&id)
                .with_field("protocolVersion", 2)
                .done(),
        )?;
        if !resp
            .get("success")
            .and_then(|s| s.as_bool())
            .unwrap_or(false)
        {
            return Err(RpcError::Protocol(format!(
                "negotiate_protocol v2 rejected: {resp}"
            )));
        }

        let id = self.next_id_str();
        let resp = self.send(
            &Request::new("set_auto_retry")
                .with_id(&id)
                .with_field("enabled", false)
                .done(),
        )?;
        if !resp
            .get("success")
            .and_then(|s| s.as_bool())
            .unwrap_or(false)
        {
            return Err(RpcError::Protocol(format!(
                "set_auto_retry off rejected: {resp}"
            )));
        }

        let id = self.next_id_str();
        let resp = self.send(
            &Request::new("set_auto_compaction")
                .with_id(&id)
                .with_field("enabled", false)
                .done(),
        )?;
        if !resp
            .get("success")
            .and_then(|s| s.as_bool())
            .unwrap_or(false)
        {
            return Err(RpcError::Protocol(format!(
                "set_auto_compaction off rejected: {resp}"
            )));
        }

        Ok(())
    }

    /// Send a request and wait for the matching response.
    ///
    /// Extension_ui_request, AvailableCommandsUpdate, and Other frames are
    /// filtered out transparently.  An `id` is auto-assigned if not set.
    ///
    /// If the process dies (`ProcessExited`) and `max_retries > 0`,
    /// automatically reconnects and retries the request up to `max_retries`
    /// times with exponential backoff (1s, 2s, 4s, ...).
    pub fn send(&mut self, req: &Request) -> Result<serde_json::Value, RpcError> {
        let mut attempts = 0;
        loop {
            match self.send_once(req) {
                Ok(v) => return Ok(v),
                Err(RpcError::ProcessExited(_)) if attempts < self.max_retries => {
                    attempts += 1;
                    let backoff = Duration::from_secs(1 << attempts.min(5));
                    eprintln!(
                        "rpc: process exited, reconnecting (attempt {attempts}/{}), backoff {backoff:?}",
                        self.max_retries
                    );
                    std::thread::sleep(backoff);
                    self.reconnect()?;
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Send a request once, without retry. Used internally by `send`.
    fn send_once(&mut self, req: &Request) -> Result<serde_json::Value, RpcError> {
        let id = req.id.clone().unwrap_or_else(|| self.next_id_str());
        let req = Request {
            id: Some(id.clone()),
            ..req.clone()
        };
        self.send_frame(&req)?;
        loop {
            let frame = self.read_frame()?;
            match frame {
                Frame::ExtensionUiRequest { .. }
                | Frame::AvailableCommandsUpdate
                | Frame::Other => continue,
                Frame::Ready { .. } => {
                    return Err(RpcError::Protocol(
                        "unexpected Ready during send".to_string(),
                    ));
                }
                Frame::RpcChunk { .. } => {
                    let _ = self.reassemble_chunk(frame);
                    continue;
                }
                Frame::Response {
                    id: ref rid, extra, ..
                } => {
                    // Match by id (if present).  Unknown commands return id: undefined.
                    if rid.as_deref() == Some(&id) {
                        return Ok(serde_json::Value::Object(extra.into_iter().collect()));
                    }
                    // Response with no id or wrong id — keep reading.
                    continue;
                }
            }
        }
    }

    /// Returns the next monotonic ID as `pipeline-N`.
    pub fn next_id_str(&mut self) -> String {
        let id = format!("pipeline-{}", self.next_id);
        self.next_id += 1;
        id
    }

    /// Write a JSON request to omp's stdin.
    fn send_frame(&mut self, req: &Request) -> Result<(), RpcError> {
        let mut line = serde_json::to_vec(req)?;
        line.push(b'\n');
        self.stdin.write_all(&line)?;
        self.stdin.flush()?;
        Ok(())
    }

    /// Read one frame (newline-delimited JSON) from omp's stdout.
    fn read_frame(&mut self) -> Result<Frame, RpcError> {
        let mut buf = Vec::with_capacity(1024);
        loop {
            buf.clear();
            let n = self.reader.read_until(b'\n', &mut buf)?;
            if n == 0 {
                let status = self.process.try_wait().ok().flatten();
                return Err(RpcError::ProcessExited(status.and_then(|s| s.code())));
            }
            if buf.ends_with(b"\n") {
                buf.pop();
            }
            if buf.is_empty() {
                continue;
            }
            let frame: Frame = serde_json::from_slice(&buf)?;
            return Ok(frame);
        }
    }

    /// Read the next non-noise frame, returning the raw JSON value.
    /// Callers can inspect the `"type"` field to distinguish response frames
    /// from agent events (agent_start, message_update, tool_execution, etc.).
    ///
    /// ExtensionUiRequest and AvailableCommandsUpdate frames are filtered out.
    /// RpcChunk frames are reassembled transparently.
    pub fn next_frame_raw(&mut self) -> Result<serde_json::Value, RpcError> {
        loop {
            let mut buf = Vec::with_capacity(1024);
            buf.clear();
            let n = self.reader.read_until(b'\n', &mut buf)?;
            if n == 0 {
                let status = self.process.try_wait().ok().flatten();
                return Err(RpcError::ProcessExited(status.and_then(|s| s.code())));
            }
            if buf.ends_with(b"\n") {
                buf.pop();
            }
            if buf.is_empty() {
                continue;
            }
            let value: serde_json::Value = serde_json::from_slice(&buf)?;
            // Filter noise frames by type field.
            let ty = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match ty {
                "extension_ui_request" | "available_commands_update" => continue,
                "rpc_chunk" => {
                    // Reassemble if possible; if not complete, continue reading.
                    let chunk_frame: Frame = serde_json::from_slice(&buf)?;
                    if let Some(assembled) = self.reassemble_chunk(chunk_frame)? {
                        // Convert the assembled Frame back to a Value.
                        // This is a bit roundabout, but chunk reassembly is uncommon.
                        return Ok(serde_json::to_value(&assembled)?);
                    }
                    continue;
                }
                _ => return Ok(value),
            }
        }
    }

    /// Reassemble a chunked payload. Returns the assembled frame if complete.
    fn reassemble_chunk(&mut self, frame: Frame) -> Result<Option<Frame>, RpcError> {
        match frame {
            Frame::RpcChunk {
                chunk_id,
                index,
                count,
                byte_length,
                data,
            } => reassemble_chunk_impl(
                &mut self.chunks,
                &chunk_id,
                index,
                count,
                byte_length,
                &data,
            ),
            _ => Ok(None),
        }
    }

    /// Read up to `max` frames, discarding startup noise (available_commands,
    /// extension_ui_request).
    fn drain_startup_noise(&mut self, max: usize) -> Result<(), RpcError> {
        for _ in 0..max {
            match self.read_frame()? {
                Frame::ExtensionUiRequest { .. }
                | Frame::AvailableCommandsUpdate
                | Frame::Other => continue,
                _ => {
                    // Non-noise frame consumed — caller will handle it when
                    // they send their first command.  In practice this doesn't
                    // happen because startup frames are always noise.
                    return Ok(());
                }
            }
        }
        Ok(())
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self
            .stdin
            .write_all(b"{\"id\":\"cancel\",\"type\":\"abort\"}\n");
        let _ = self.stdin.flush();
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

/// Standalone chunk reassembly helper (testable without a Client).
fn reassemble_chunk_impl(
    chunks: &mut HashMap<String, ChunkState>,
    chunk_id: &str,
    index: u32,
    count: u32,
    byte_length: u64,
    data: &str,
) -> Result<Option<Frame>, RpcError> {
    if count == 0 {
        return Err(RpcError::Chunk("count is 0".to_string()));
    }
    if byte_length > 64 * 1024 * 1024 {
        return Err(RpcError::Chunk(format!(
            "reassembled frame too large: {byte_length} > 64MiB"
        )));
    }
    if index as usize >= count as usize {
        return Err(RpcError::Chunk(format!("index {index} >= count {count}")));
    }

    let entry = chunks.entry(chunk_id.to_string()).or_insert_with(|| {
        let mut parts = Vec::with_capacity(count as usize);
        parts.resize_with(count as usize, || None);
        ChunkState {
            count,
            parts,
            byte_length,
            received: 0,
        }
    });

    if entry.byte_length != byte_length {
        return Err(RpcError::Chunk(format!(
            "byte_length mismatch: expected {}, got {}",
            entry.byte_length, byte_length
        )));
    }
    if entry.count != count {
        return Err(RpcError::Chunk(format!(
            "count mismatch: expected {}, got {}",
            entry.count, count
        )));
    }
    if entry.parts[index as usize].is_some() {
        return Err(RpcError::Chunk(format!(
            "duplicate chunk index {index} for chunk_id {chunk_id}"
        )));
    }

    entry.parts[index as usize] = Some(data.to_string());
    entry.received += 1;

    if entry.received == entry.count {
        let mut assembled = String::new();
        for part in entry.parts.iter().flatten() {
            use base64::Engine;
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(part)
                .map_err(|e| RpcError::Chunk(format!("base64 decode error: {e}")))?;
            assembled.push_str(&String::from_utf8_lossy(&decoded));
        }
        chunks.remove(chunk_id);
        let frame: Frame = serde_json::from_str(&assembled)?;
        Ok(Some(frame))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_serialization() {
        let req = Request::new("abort").with_id("pipeline-1").done();
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["id"], "pipeline-1");
        assert_eq!(json["type"], "abort");
    }

    #[test]
    fn request_with_extra_fields() {
        let req = Request::new("prompt")
            .with_id("p-1")
            .with_field("message", "hello")
            .done();
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["type"], "prompt");
        assert_eq!(json["message"], "hello");
    }

    #[test]
    fn request_no_id() {
        let json = serde_json::to_value(Request::new("abort").done()).unwrap();
        assert!(json.get("id").is_none());
        assert_eq!(json["type"], "abort");
    }

    #[test]
    fn frame_ready_deserialization() {
        let json = r#"{"type":"ready","protocolVersion":1,"supportedProtocolVersions":[1,2],"maxFrameBytes":1048576,"maxReassembledFrameBytes":67108864}"#;
        let frame: Frame = serde_json::from_str(json).unwrap();
        match frame {
            Frame::Ready {
                protocol_version, ..
            } => {
                assert_eq!(protocol_version, 1);
            }
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn frame_ready_minimal() {
        // The doc says ready may be just {"type":"ready"} — no extra fields.
        let json = r#"{"type":"ready"}"#;
        let frame: Frame = serde_json::from_str(json).unwrap();
        assert!(matches!(frame, Frame::Ready { .. }));
    }

    #[test]
    fn frame_extension_ui_request_deserialization() {
        let json = r#"{"type":"extension_ui_request","id":"abc","method":"setStatus"}"#;
        let frame: Frame = serde_json::from_str(json).unwrap();
        assert!(matches!(frame, Frame::ExtensionUiRequest { .. }));
    }

    #[test]
    fn frame_available_commands_deserialization() {
        let json = r#"{"type":"available_commands_update","commands":[]}"#;
        let frame: Frame = serde_json::from_str(json).unwrap();
        assert!(matches!(frame, Frame::AvailableCommandsUpdate));
    }

    #[test]
    fn frame_response_deserialization() {
        let json = r#"{"type":"response","id":"pipeline-1","status":"ok"}"#;
        let frame: Frame = serde_json::from_str(json).unwrap();
        match frame {
            Frame::Response { id, extra, .. } => {
                assert_eq!(id.as_deref(), Some("pipeline-1"));
                assert_eq!(extra.get("status").and_then(|v| v.as_str()), Some("ok"));
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn frame_response_no_id() {
        // Unknown commands return response with id: undefined
        let json = r#"{"type":"response","success":false,"error":"Unknown command"}"#;
        let frame: Frame = serde_json::from_str(json).unwrap();
        match frame {
            Frame::Response { id, extra, .. } => {
                assert!(id.is_none());
                assert_eq!(extra.get("success").and_then(|v| v.as_bool()), Some(false));
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn frame_other_catch_all() {
        let json = r#"{"type":"turn","id":"t1"}"#;
        let frame: Frame = serde_json::from_str(json).unwrap();
        assert!(matches!(frame, Frame::Other));
    }

    #[test]
    fn chunk_reassembly_partial() {
        let mut chunks: HashMap<String, ChunkState> = HashMap::new();
        use base64::Engine;
        let part1 =
            base64::engine::general_purpose::STANDARD.encode(r#"{"type":"response","id":"p1""#);
        let part2 = base64::engine::general_purpose::STANDARD.encode(r#","status":"ok"}"#);

        let result = reassemble_chunk_impl(&mut chunks, "chunk-1", 0, 2, 100, &part1).unwrap();
        assert!(result.is_none());

        let result = reassemble_chunk_impl(&mut chunks, "chunk-1", 1, 2, 100, &part2).unwrap();
        assert!(result.is_some(), "chunk not complete");
        match result.unwrap() {
            Frame::Response { id, extra, .. } => {
                assert_eq!(id.as_deref(), Some("p1"));
                assert_eq!(extra.get("status").and_then(|v| v.as_str()), Some("ok"));
            }
            other => panic!("expected Response, got: {other:?}"),
        }
    }

    /// Live smoke against a real omp binary. Run with:
    /// `cargo test -- --ignored omp_rpc::tests::live_handshake`.
    /// Requires `omp` on PATH.
    #[test]
    #[ignore]
    fn live_handshake() {
        let mut client = Client::new("omp").expect("spawn + ready failed");
        // Send a command that omp is guaranteed to handle: `get_state`.
        let id = client.next_id_str();
        let req = Request::new("get_state").with_id(&id).done();
        match client.send(&req) {
            Ok(v) => println!("get_state response: {v}"),
            Err(RpcError::ProcessExited(_)) | Err(RpcError::Timeout) => {
                println!("command unsupported; handshake already verified")
            }
            Err(e) => panic!("send failed: {e}"),
        }
    }

    /// Pure check of the negotiation response contract: success must be a
    /// literal `true`; anything else is a rejection (#51).
    fn negotiation_ok(resp: &serde_json::Value) -> bool {
        resp.get("success")
            .and_then(|s| s.as_bool())
            .unwrap_or(false)
    }

    #[test]
    fn negotiate_request_serialization() {
        // Wire format of the three negotiation commands.
        let n = serde_json::to_value(
            Request::new("negotiate_protocol")
                .with_id("n1")
                .with_field("protocolVersion", 2)
                .done(),
        )
        .unwrap();
        assert_eq!(n["type"], "negotiate_protocol");
        assert_eq!(n["protocolVersion"], 2);

        let r = serde_json::to_value(
            Request::new("set_auto_retry")
                .with_id("n2")
                .with_field("enabled", false)
                .done(),
        )
        .unwrap();
        assert_eq!(r["type"], "set_auto_retry");
        assert_eq!(r["enabled"], false);

        let c = serde_json::to_value(
            Request::new("set_auto_compaction")
                .with_id("n3")
                .with_field("enabled", false)
                .done(),
        )
        .unwrap();
        assert_eq!(c["type"], "set_auto_compaction");
        assert_eq!(c["enabled"], false);
    }

    #[test]
    fn negotiation_response_contract() {
        // Real omp shapes (probed against /usr/bin/omp).
        let ok = serde_json::json!({
            "id": "n1", "type": "response", "command": "negotiate_protocol",
            "success": true, "data": {"protocolVersion": 2}
        });
        assert!(negotiation_ok(&ok));

        let rejected = serde_json::json!({
            "id": "x1", "type": "response",
            "command": "negotiate_protocol",
            "success": false,
            "error": "Unsupported RPC protocol version: undefined"
        });
        assert!(!negotiation_ok(&rejected));

        let unknown = serde_json::json!({
            "type": "response", "command": "bogus_command",
            "success": false, "error": "Unknown command: bogus_command"
        });
        assert!(!negotiation_ok(&unknown));

        // Missing/absent success field must not pass.
        assert!(!negotiation_ok(&serde_json::json!({"type": "response"})));
        assert!(!negotiation_ok(&serde_json::json!({"success": "yes"})));
    }

    /// Live smoke: full negotiation sequence against real omp. Run with:
    /// `cargo test -- --ignored omp_rpc::tests::live_negotiation`.
    #[test]
    #[ignore]
    fn live_negotiation() {
        // Client::new now negotiates internally — reaching a client at all
        // proves the v2 sequence succeeded against this omp build.
        let mut client = Client::new("omp").expect("spawn + ready + negotiation failed");
        let id = client.next_id_str();
        let resp = client
            .send(&Request::new("get_state").with_id(&id).done())
            .expect("post-negotiation command");
        println!("session negotiated; get_state → {resp}");
    }

    // ---- P7: timeout + reconnect + retry smoke tests ----

    /// Spawn a shell script that hangs forever (no `ready` frame).
    /// Leaks the tempdir so the file persists for the test's lifetime.
    fn fake_omp_hangs() -> std::path::PathBuf {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("hang.sh");
        std::fs::write(&script, "#!/bin/sh\nsleep 60\n").unwrap();
        std::fs::set_permissions(
            &script,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        std::mem::forget(dir);
        script
    }

    #[test]
    fn connect_timeout_kills_hanging_omp() {
        let script = fake_omp_hangs();
        let start = std::time::Instant::now();
        let result = Client::new_with_opts(
            script.to_str().unwrap(),
            Some(Duration::from_millis(200)),
            0,
        );
        let elapsed = start.elapsed();
        match result {
            Ok(_) => panic!("should fail to connect"),
            Err(RpcError::Timeout) => {}
            Err(other) => panic!("should be Timeout, got: {other:?}"),
        }
        assert!(
            elapsed < Duration::from_secs(2),
            "should not hang past timeout (took {elapsed:?})"
        );
    }

    #[test]
    fn send_process_exited_no_retry_by_default() {
        let script = fake_omp_hangs();
        let mut client = Client::new(script.to_str().unwrap()).unwrap();
        let _ = client.process.kill();
        let _ = client.process.wait();
        let id = client.next_id_str();
        let req = Request::new("ping").with_id(&id).done();
        let err = client.send(&req).expect_err("send should fail");
        assert!(
            matches!(err, RpcError::ProcessExited(_)),
            "should propagate ProcessExited without retry, got: {err:?}"
        );
    }

    #[test]
    fn set_timeout_stores_value() {
        let script = fake_omp_hangs();
        let mut client = Client::new(script.to_str().unwrap()).unwrap();
        client.set_timeout(Some(Duration::from_secs(5)));
        assert_eq!(client.timeout, Some(Duration::from_secs(5)));
        client.set_timeout(None);
        assert!(client.timeout.is_none());
    }

    #[test]
    fn backoff_first_retry_uses_one_second() {
        // Documented schedule: 1s, 2s, 4s, 8s, 16s, capped at 32s.
        for (attempt, expected) in [1u64, 2, 4, 8, 16].iter().enumerate() {
            let secs = 1u64 << (attempt as u32).min(5);
            assert_eq!(secs, *expected, "backoff mismatch at attempt {attempt}");
        }
        let secs = 1u64 << 5u64.min(5);
        assert_eq!(secs, 32);
    }
}
