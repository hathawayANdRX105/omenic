//! G6 e2e — the real daemon path, not the shadow path.
//!
//! `orbit_container.rs` rebuilds `Daemon::orbit_setup`'s resolution logic in
//! the test itself ("performs in production" — its own words) and hands the
//! result to a `Worker` directly. That skips exactly the pieces G6 wired:
//! `Daemon::start` -> `assemble_plugins` -> `orbit_setup` -> real worker ->
//! real orbit loop -> `HttpLlm` -> an actual HTTP call.
//!
//! These tests close that gap. A daemon is started for real on a temp socket
//! with `orbit_model` pointing at a local mock OpenAI server, so the request
//! the loop makes leaves the process, hits a socket we control, and the reply
//! travels back through the real SSE/HTTP decode path. What is asserted is
//! what only the full chain can prove:
//!
//! 1. `AGENTS.md` in the workspace the daemon was configured with reaches the
//!    HTTP body the backend actually received — end to end, not a recorded
//!    `Context` from a scripted backend swapped in at construction.
//! 2. `[daemon] max_turns` in the config document caps the real run: the mock
//!    never ends a turn on its own, so the only thing that can stop the loop
//!    is the cap flowing document -> assemble -> resolve -> worker -> loop.
//! 3. A daemon killed mid-run, restarted on the same DB, closes the orphan.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use daemon::protocol::Command;
use daemon::{Daemon, DaemonClient, DaemonConfig, EventFrame, Subscription};
use serde_json::{Value, json};
use tempfile::tempdir;

/// Distinctive token written into the workspace `AGENTS.md`. If instruction
/// injection works end to end, this string is inside the HTTP request body
/// the mock server received.
const MARKER: &str = "G6-E2E-INSTRUCTION-MARKER";

const EVENT_TIMEOUT: Duration = Duration::from_secs(10);

/// A tiny OpenAI-compatible server: one thread per connection, replays the
/// canned SSE stream for every `/chat/completions` POST, and records the raw
/// request bodies so the test can assert on what the loop really sent.
struct MockOpenAi {
    addr: String,
    bodies: Arc<Mutex<Vec<String>>>,
}

impl MockOpenAi {
    fn start(replies: Vec<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
        let addr = listener.local_addr().expect("local addr");
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let bodies_srv = Arc::clone(&bodies);
        let replies_srv = replies.clone();

        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let bodies = Arc::clone(&bodies_srv);
                let replies = replies_srv.clone();
                thread::spawn(move || serve(stream, bodies, replies));
            }
        });

        MockOpenAi {
            addr: format!("http://127.0.0.1:{}", addr.port()),
            bodies,
        }
    }

    /// Bodies of every `/chat/completions` POST, in arrival order.
    fn received(&self) -> Vec<String> {
        self.bodies.lock().expect("bodies").clone()
    }
}

fn serve(stream: TcpStream, bodies: Arc<Mutex<Vec<String>>>, replies: Vec<String>) {
    let mut stream = stream;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));

    // Read the request: headers until blank line, then Content-Length bytes.
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1];
    while let Ok(n) = stream.read(&mut tmp) {
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let head = String::from_utf8_lossy(&buf).to_string();
    let body_len = head
        .split("\r\n")
        .find_map(|l| {
            l.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .and_then(|v| v.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    let mut body = buf;
    while body.len() < head.len() + body_len {
        let mut chunk = vec![0u8; head.len() + body_len - body.len()];
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
    }
    let payload = &body[head.len()..];
    let payload_str = String::from_utf8_lossy(payload).to_string();
    if head.starts_with("POST") && head.contains("/chat/completions") {
        bodies.lock().expect("bodies").push(payload_str);
    }

    let idx = bodies.lock().expect("bodies").len().saturating_sub(1);
    let reply = replies
        .get(idx)
        .or_else(|| replies.last())
        .cloned()
        .unwrap_or_else(minimal_reply);

    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        reply.len(),
        reply
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

fn minimal_reply() -> String {
    sse_text("hello from the mock backend", true)
}

/// One SSE `data:` line carrying a chat-completion chunk with a text delta.
fn sse_text(delta: &str, finish: bool) -> String {
    let reason = if finish {
        serde_json::Value::String("stop".into())
    } else {
        serde_json::Value::Null
    };
    let chunk = json!({
        "choices": [{
            "delta": { "content": delta },
            "finish_reason": reason
        }]
    });
    format!("data: {}\n\n", chunk)
}

/// A turn that emits `delta` and finishes — enough for a one-round run.
fn one_text_turn(delta: &str) -> String {
    sse_text(delta, true)
}

/// `max_turns` test: every reply carries a tool call, so each round ends with
/// a dispatch and the loop continues into the next round. A reply with no
/// tool calls would let the loop exit on a plain EndTurn and the cap would
/// never be reached. The tool name is one the real catalog does not provide —
/// the loop routes that to an error result, which still counts as a completed
/// round and keeps the loop spinning. The only thing that can end this run is
/// `LoopConfig::max_turns`.
fn endless_turns(n: usize) -> Vec<String> {
    (0..n)
        .map(|i| {
            // First chunk: the tool call. Second chunk: finish without text,
            // which closes the round and lets the loop dispatch + continue.
            let call = json!({
                "choices": [{
                    "delta": {
                        "role": "assistant",
                        "tool_calls": [{
                            "index": 0,
                            "id": format!("call_{i}"),
                            "type": "function",
                            "function": { "name": "g6_e2e_nonexistent", "arguments": "{}" }
                        }]
                    },
                    "finish_reason": serde_json::Value::Null
                }]
            });
            let finish = json!({
                "choices": [{
                    "delta": {},
                    "finish_reason": "tool_calls"
                }]
            });
            format!("data: {}\ndata: {}\n\n", call, finish)
        })
        .collect()
}

fn daemon_cfg(dir: &std::path::Path, mock: &MockOpenAi, max_turns: usize) -> DaemonConfig {
    DaemonConfig {
        socket_path: Some(dir.join("daemon.sock")),
        omp_path: "omp".into(),
        session_db_path: Some(dir.join("sessions.db")),
        orbit_model: Some(adaptor::Model {
            api_key: "test-key".into(),
            model: "g6-e2e".into(),
            base_url: Some(mock.addr.clone()),
            max_tokens: Some(1024),
        }),
        // The workspace the loop searches for AGENTS.md.
        cwd: dir.to_path_buf(),
        max_turns,
        mcp_servers: Vec::new(),
        llm_fallbacks: Vec::new(),
    }
}

fn workspace_with_agents_md(dir: &std::path::Path) {
    std::fs::write(
        dir.join("AGENTS.md"),
        format!("# Test workspace\n\nRule: emit {MARKER} in every reply.\n"),
    )
    .expect("write AGENTS.md");
}

/// Drain worker events until the stream goes quiet. Each `next_event` already
/// bounds its own wait, so this returns as soon as the daemon stops pushing.
fn drain_events(sub: &mut Subscription) -> Vec<EventFrame> {
    let mut out = Vec::new();
    loop {
        match sub
            .next_event(EVENT_TIMEOUT)
            .expect("subscription readable")
        {
            Some(frame) => {
                assert_eq!(frame.topic, "worker", "pushed on the subscribed topic");
                out.push(frame);
            }
            None => return out,
        }
    }
}

fn prompt(client: &DaemonClient, message: &str) {
    let resp = client
        .call_raw(Command::WorkerPrompt, json!({ "message": message }))
        .expect("prompt via daemon");
    assert!(resp.success, "prompt failed: {:?}", resp.error);
}

/// ① AGENTS.md travels workspace -> daemon config -> assemble -> orbit_setup
/// -> loop -> HTTP body. The assertion is on the bytes the mock received,
/// which no unit test along the way can forge.
#[test]
fn agents_md_reaches_the_real_http_request() {
    let dir = tempdir().expect("temp dir");
    workspace_with_agents_md(dir.path());

    let mock = MockOpenAi::start(vec![one_text_turn("ack")]);
    let cfg = daemon_cfg(dir.path(), &mock, 8);
    let mut daemon = Daemon::start(cfg).expect("daemon start");

    let client = DaemonClient::connect_to(daemon.socket_addr().path());
    let mut sub = client.subscribe("worker").expect("subscribe");
    prompt(&client, "ping");
    drain_events(&mut sub);

    let bodies = mock.received();
    assert!(!bodies.is_empty(), "the loop made at least one HTTP call");

    let joined = bodies.join("\n---\n");
    assert!(
        joined.contains(MARKER),
        "AGENTS.md marker missing from the real request body"
    );

    drop(client);
    daemon.shutdown();
}

/// ② `max_turns` in the config document caps the real run. The mock never
/// finishes a turn, so a working cap is the only way this test terminates
/// without timing out.
#[test]
fn max_turns_from_config_caps_the_real_run() {
    let dir = tempdir().expect("temp dir");
    std::fs::write(dir.path().join("AGENTS.md"), "# bare\n").expect("AGENTS.md");

    let cap = 3;
    // More rounds than the cap; each is an unfinished assistant turn.
    let mock = MockOpenAi::start(endless_turns(12));
    let cfg = daemon_cfg(dir.path(), &mock, cap);
    let mut daemon = Daemon::start(cfg).expect("daemon start");

    let client = DaemonClient::connect_to(daemon.socket_addr().path());
    let mut sub = client.subscribe("worker").expect("subscribe");
    prompt(&client, "go");

    let events = drain_events(&mut sub);
    let starts = events
        .iter()
        .filter(|f| f.event.get("event").and_then(Value::as_str) == Some("agent_start"))
        .count();

    // The loop runs exactly `cap` rounds and then stops on the cap.
    assert_eq!(
        starts, cap,
        "the run must stop at max_turns={cap}, saw {starts} agent_start events"
    );

    drop(client);
    daemon.shutdown();
}

/// ③ A daemon killed mid-run leaves an orphan; the next start closes it.
/// This exercises the full `Daemon::start` repair path over a real DB file.
#[test]
fn restart_closes_orphaned_run() {
    let dir = tempdir().expect("temp dir");
    let db_path = dir.path().join("sessions.db");

    // Process one: a run starts and the process dies before its TurnEnd.
    {
        let db = session::SessionDb::open(&db_path).expect("open db");
        db.ensure_session("s1", "unfinished").expect("session row");
        db.append_turn_log(
            "s1",
            &[session::TurnRecord::TurnStart {
                run_id: "r1".into(),
                ts_ms: 1_000,
            }],
        )
        .expect("record start");
    }

    // Process two: same DB, fresh start. Repair runs before bind.
    let mock = MockOpenAi::start(vec![one_text_turn("ack")]);
    let cfg = daemon_cfg(dir.path(), &mock, 8);
    let mut daemon = Daemon::start(cfg).expect("daemon start");

    let records = daemon
        .sessions()
        .load_turn_log("s1")
        .expect("read repaired log");
    assert_eq!(records.len(), 2, "the orphan gained exactly one closer");
    match &records[1] {
        session::TurnRecord::TurnEnd { run_id, status, .. } => {
            assert_eq!(run_id, "r1");
            assert_eq!(*status, session::ABORTED, "the orphan is closed as aborted");
        }
        other => panic!("expected TurnEnd closer, got {other:?}"),
    }

    daemon.shutdown();
}
