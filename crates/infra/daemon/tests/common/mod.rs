//! Shared scaffolding for the daemon's end-to-end tests.
//!
//! Each file in `tests/` compiles as its own crate, so anything two e2e tests
//! both need lives here rather than being copied — the mock OpenAI server and
//! the daemon config in particular are the parts that must behave identically
//! across suites, or a difference in one would silently mask a failure in the
//! other.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use daemon::protocol::Command;
use daemon::{DaemonConfig, EventFrame, Subscription};
use serde_json::{Value, json};

/// How long a single event read may block before the test gives up.
pub const EVENT_TIMEOUT: Duration = Duration::from_secs(10);

/// A tiny OpenAI-compatible server: one thread per connection, replays the
/// canned SSE stream for the request whose index it is, and records every raw
/// request body so a test can assert on what the loop really sent.
pub struct MockOpenAi {
    addr: String,
    bodies: Arc<Mutex<Vec<String>>>,
}

impl MockOpenAi {
    /// `replies[i]` is served to the i-th `/chat/completions` POST; a request
    /// beyond the list gets the last entry.
    pub fn start(replies: Vec<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
        let addr = listener.local_addr().expect("local addr");
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let bodies_srv = Arc::clone(&bodies);
        let replies_srv = replies;

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

    /// Bodies of every `/chat/completions` POST, in arrival order. Request
    /// *n+1* carries the tool results of round *n*, so this is also the record
    /// of what the loop's tools actually returned to the model.
    pub fn received(&self) -> Vec<String> {
        self.bodies.lock().expect("bodies").clone()
    }

    pub fn addr(&self) -> &str {
        &self.addr
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
pub fn sse_text(delta: &str, finish: bool) -> String {
    let reason = if finish {
        Value::String("stop".into())
    } else {
        Value::Null
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
pub fn one_text_turn(delta: &str) -> String {
    sse_text(delta, true)
}

/// A daemon pointed at `mock` with a temp socket and DB in `dir`.
pub fn daemon_cfg(dir: &std::path::Path, mock: &MockOpenAi, max_turns: usize) -> DaemonConfig {
    DaemonConfig {
        socket_path: Some(dir.join("daemon.sock")),
        omp_path: "omp".into(),
        session_db_path: Some(dir.join("sessions.db")),
        orbit_model: Some(adaptor::Model {
            api_key: "test-key".into(),
            model: "g6-e2e".into(),
            base_url: Some(mock.addr().to_string()),
            max_tokens: Some(1024),
        }),
        cwd: dir.to_path_buf(),
        max_turns,
        ..Default::default()
    }
}

/// Send a prompt and let the daemon work; `drain_events` bounds each wait.
pub fn prompt(client: &daemon::DaemonClient, message: &str) {
    let resp = client
        .call_raw(Command::WorkerPrompt, json!({ "message": message }))
        .expect("prompt via daemon");
    assert!(resp.success, "prompt failed: {:?}", resp.error);
}

/// Drain worker events until the stream goes quiet. Each `next_event` bounds
/// its own wait, so this returns as soon as the daemon stops pushing.
pub fn drain_events(sub: &mut Subscription) -> Vec<EventFrame> {
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
