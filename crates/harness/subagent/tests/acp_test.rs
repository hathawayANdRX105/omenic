use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;

use omenic_harness_subagent::acp::*;
use serde_json::{Value, json};
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
        .send(format!("{{\"jsonrpc\":\"{JSONRPC}\",\"id\":999,\"result\":{{}}}}\n").into_bytes())
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
