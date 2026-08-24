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
//! let mut client = rpc::Client::new("/usr/bin/omp")?;
//! let resp = client.send(&rpc::Request::new("ping").done())?;
//! ```

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdin, Command, Stdio};

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
/// and provides `send()` for command-response exchange.
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
            .ok_or_else(|| RpcError::Protocol("failed to capture stdin".to_string()))?;
        let stdout = process
            .stdout
            .take()
            .ok_or_else(|| RpcError::Protocol("failed to capture stdout".to_string()))?;

        let mut client = Client {
            process,
            stdin,
            reader: BufReader::new(stdout),
            next_id: 1,
            chunks: HashMap::new(),
        };

        // Read the initial `ready` message.
        let ready = client.read_frame()?;
        match ready {
            Frame::Ready { .. } => { /* ok */ }
            other => {
                return Err(RpcError::Protocol(format!(
                    "expected ready, got: {other:?}"
                )));
            }
        }

        // v2 session negotiation (#51, mvp-design §6.2): pin protocol
        // semantics across omp default configs and disable background
        // retry/compaction so the runner owns the session lifecycle.
        client.negotiate_session()?;

        Ok(client)
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
    pub fn send(&mut self, req: &Request) -> Result<serde_json::Value, RpcError> {
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
    /// `cargo test -- --ignored rpc::tests::live_handshake`.
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
    /// `cargo test -- --ignored rpc::tests::live_negotiation`.
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
}
