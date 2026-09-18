//! B2a — a restarted daemon resumes the persisted session history.
//!
//! The observable contract: a daemon that dropped and restarted on the same
//! session DB feeds the worker's live orbit context from the persisted
//! messages before the prompt reaches the LLM.  The assertion is on the
//! bytes of the `/chat/completions` request the mock backend receives, which
//! only the full `Daemon::start` -> `WorkerHandle::resume_session` ->
//! `OrbitEngine::resume_orbit_context` -> `HttpLlm` chain can produce.
//!
//! The mock records every request body.  After a restart, the request for
//! the same `session_id` must contain the persisted history ("第一轮" user
//! and "第一轮回复" assistant) *ahead* of the new prompt, proving the
//! worker's context was rebuilt from the DB rather than starting blank.
//!
//! In-session dedupe is also pinned: a second prompt for the *same*
//! session on the *same* daemon must not double the history in the request
//! body (the engine's `resumed_session` gate keeps the context stable
//! across consecutive prompts).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use daemon::protocol::Command;
use daemon::{Daemon, DaemonClient, DaemonConfig, EventFrame};
use serde_json::{Value, json};
use session::SessionRole;
use tempfile::tempdir;

/// Distinctive user message that will be persisted then replayed.
const RESUME_USER: &str = "第一轮";
/// Distinctive assistant reply that will be persisted then replayed.
const RESUME_ASSISTANT: &str = "第一轮回复";
/// A second user prompt that arrives *after* the history has been appended.
const FRESH_PROMPT: &str = "第二轮";
/// A third prompt on the same daemon + session: the dedupe gate must keep
/// the request's `messages` array from re-appending the persisted history.
const THIRD_PROMPT: &str = "第三轮";

const EVENT_TIMEOUT: Duration = Duration::from_secs(10);

/// A tiny OpenAI-compatible server: one thread per connection, replays a
/// canned SSE stream for every `/chat/completions` POST, and records the
/// raw request bodies so the test can assert on what the loop really sent.
/// Shape mirrors `g6_e2e.rs`'s `MockOpenAi`.
struct MockOpenAi {
    addr: String,
    bodies: Arc<Mutex<Vec<String>>>,
}

impl MockOpenAi {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
        let addr = listener.local_addr().expect("local addr");
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let bodies_srv = Arc::clone(&bodies);

        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let bodies = Arc::clone(&bodies_srv);
                thread::spawn(move || serve(stream, bodies));
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

fn serve(stream: TcpStream, bodies: Arc<Mutex<Vec<String>>>) {
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

    let reply = sse_text("ok", true);
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        reply.len(),
        reply,
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

/// One SSE `data:` line carrying a chat-completion chunk with a text delta
/// and a finish — enough to close a single round cleanly.
fn sse_text(delta: &str, finish: bool) -> String {
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

fn daemon_cfg(dir: &std::path::Path, mock: &MockOpenAi) -> DaemonConfig {
    DaemonConfig {
        socket_path: Some(dir.join("daemon.sock")),
        omp_path: "omp".into(),
        session_db_path: Some(dir.join("sessions.db")),
        orbit_model: Some(adaptor::Model {
            api_key: "test-key".into(),
            model: "b2a-resume".into(),
            base_url: Some(mock.addr.clone()),
            max_tokens: Some(1024),
        }),
        cwd: dir.to_path_buf(),
        max_turns: 4,
        mcp_servers: Vec::new(),
        llm_fallbacks: Vec::new(),
    }
}

/// Extract the `messages` array from an OpenAI `/chat/completions` body and
/// return the per-entry `{role, content}` pairs as strings.  Returns the
/// raw JSON body verbatim on parse failure so the caller can assert on the
/// shape itself.
fn body_messages(body: &str) -> Vec<String> {
    let v: Value =
        serde_json::from_str(body).unwrap_or_else(|e| panic!("request body not JSON: {e}"));
    let arr = v
        .get("messages")
        .and_then(Value::as_array)
        .expect("the OpenAI request body must carry a `messages` array");
    arr.iter()
        .map(|m| {
            let role = m
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("<missing role>");
            let content = m.get("content").map(|c| c.to_string()).unwrap_or_default();
            format!("{role} {content}")
        })
        .collect()
}

/// Pump events until `agent_end` lands or the timeout expires.  Orbit turns
/// are async (the prompt only acks queueing), so the mock's request body is
/// only guaranteed once the loop has made its LLM round.
fn drain_until_agent_end(sub: &mut daemon::Subscription) -> Vec<EventFrame> {
    let mut out = Vec::new();
    let deadline = std::time::Instant::now() + EVENT_TIMEOUT;
    loop {
        if std::time::Instant::now() >= deadline {
            panic!("agent_end never arrived within {EVENT_TIMEOUT:?}");
        }
        match sub
            .next_event(Duration::from_millis(200))
            .expect("subscription readable")
        {
            Some(frame) => {
                let tag = frame
                    .event
                    .get("event")
                    .and_then(Value::as_str)
                    .unwrap_or("<missing>")
                    .to_string();
                out.push(frame);
                if tag == "agent_end" {
                    return out;
                }
            }
            None => continue,
        }
    }
}

#[test]
fn restart_resumes_session_history_into_the_llm_request() {
    let dir = tempdir().expect("temp dir");
    // A bare AGENTS.md keeps the instruction walk fast and the test
    // deterministic across repos (the marker below is what we assert on).
    std::fs::write(dir.path().join("AGENTS.md"), "# bare\n").expect("AGENTS.md");
    let mock = MockOpenAi::start();

    // Both "processes" share one socket path; the restart model is
    // shutdown process 1's daemon explicitly, then start process 2 on the
    // same socket + DB file.
    let socket = dir.path().join("daemon.sock");

    // ---- Process 1: create the session + history, run one turn, restart.
    //
    // NOTE: `daemon.shutdown()` is called *inside* the block, before the
    // binding goes out of scope. `Drop` for `Daemon` is documented, but
    // shutdown (accept-loop join + lock release + listener cleanup) happens
    // explicitly so process 1's InstanceLock is guaranteed released before
    // process 2 binds the same socket — no race on the flock.
    let daemon1 = {
        let cfg = daemon_cfg(dir.path(), &mock);
        let mut daemon = Daemon::start(cfg).expect("daemon start (1)");
        let client = DaemonClient::connect_to(&socket);
        let _ = client
            .session_create("s-b2a", "resume session")
            .expect("session.create");
        let _ = client
            .session_append("s-b2a", SessionRole::User, RESUME_USER)
            .expect("session.append (user)");
        let _ = client
            .session_append("s-b2a", SessionRole::Assistant, RESUME_ASSISTANT)
            .expect("session.append (assistant)");

        // First prompt on process 1: the engine was just spawned, so this
        // is the first replay for this session — the two persisted rows get
        // appended to the fresh context ahead of "first prompt".
        let mut sub = client.subscribe("worker").expect("subscribe");
        prompt_with_session(&client, "first prompt");
        let _ = drain_until_agent_end(&mut sub);

        // Drop the client, then shut down the daemon explicitly so the
        // accept loop has joined and the InstanceLock is released before
        // the block ends — no flock race with process 2.
        drop(client);
        daemon.shutdown();
        daemon
    };
    drop(daemon1);

    // Snapshot: how many LLM requests have reached the mock so far.  This
    // is the "before restart" count; any request the second process makes
    // must be *after* this index.
    let bodies_before_restart = mock.received().len();

    // ---- Process 2: same socket + DB, the engine starts fresh.
    let daemon2 = {
        let cfg = daemon_cfg(dir.path(), &mock);
        let mut daemon = Daemon::start(cfg).expect("daemon start (2)");

        let client = DaemonClient::connect_to(&socket);
        let mut sub = client.subscribe("worker").expect("subscribe");

        // Same session, fresh prompt.  The new engine's context is empty,
        // so the B2a path must load the two persisted rows and replay them
        // into `ctx.messages` before the prompt goes out.
        prompt_with_session(&client, FRESH_PROMPT);
        let _ = drain_until_agent_end(&mut sub);

        // Third prompt, same daemon + same session: the engine's
        // `resumed_session` gate must block a second replay of the
        // persisted history.  The request still carries FRESH_PROMPT in
        // its live context, but the persisted rows reach the LLM body
        // exactly once.
        prompt_with_session(&client, THIRD_PROMPT);
        let _ = drain_until_agent_end(&mut sub);

        drop(client);
        daemon.shutdown();
        daemon
    };
    drop(daemon2);

    // ---- Assert: the post-restart LLM request carries the persisted
    // history *in order* ahead of the fresh prompt.
    let bodies = mock.received();
    assert!(
        bodies.len() > bodies_before_restart,
        "the restarted daemon made at least one new LLM request (had {} before, {} after)",
        bodies_before_restart,
        bodies.len(),
    );
    let post_restart: Vec<String> = bodies[bodies_before_restart..].to_vec();
    let msgs: Vec<Vec<String>> = post_restart.iter().map(|b| body_messages(b)).collect();
    // Flatten every post-restart request's message list; the resumed rows
    // must appear *ahead of* the fresh prompt in the same request.
    let flat: Vec<String> = msgs.into_iter().flatten().collect();
    let joined = post_restart.join("\n---\n");
    let positions = |needle: &str| -> Vec<usize> {
        flat.iter()
            .enumerate()
            .filter(|(_, s)| s.contains(needle))
            .map(|(i, _)| i)
            .collect()
    };
    let user_hist = positions(RESUME_USER);
    let ass_hist = positions(RESUME_ASSISTANT);
    let fresh = positions(FRESH_PROMPT);
    assert!(
        !user_hist.is_empty() && !ass_hist.is_empty(),
        "resumed history must reach the LLM body, saw post-restart messages: {joined:?}"
    );
    assert!(
        !fresh.is_empty(),
        "the fresh prompt must reach the LLM body, saw post-restart messages: {joined:?}"
    );
    assert!(
        user_hist[0] < fresh[0] && ass_hist[0] < fresh[0],
        "history (user at {user_hist:?}, assistant at {ass_hist:?}) must precede the fresh prompt ({fresh:?}) in the request"
    );
    assert!(
        user_hist[0] < ass_hist[0],
        "the persisted user row must precede the persisted assistant row (seq order)"
    );

    // ---- Assert dedupe: the third request on the *same* engine must
    // re-send the live context (which already holds the replayed rows)
    // without appending the persisted history a second time.
    let dedupe_msgs: Vec<Vec<String>> = post_restart.iter().map(|b| body_messages(b)).collect();
    assert!(
        dedupe_msgs.len() >= 2,
        "the restart+third-prompt flow must reach the mock twice, saw {}",
        dedupe_msgs.len(),
    );
    // Dedupe: the *second* post-restart request (the third prompt overall)
    // carries the live context — system + the two replayed rows + both
    // prompts and their "ok" replies — exactly once each.  If the engine
    // re-replayed the persisted history on the repeated `resume_session`
    // call, "第一轮" / "第一轮回复" would each appear *twice* here.  (The
    // user row and its reply are counted together: `contains("第一轮")`
    // matches the assistant row too, so the expected live-context count is
    // 2, and a double replay would make it 4.)
    let second_req = &dedupe_msgs[dedupe_msgs.len() - 1];
    let third_hits: Vec<&String> = second_req
        .iter()
        .filter(|m| m.contains(THIRD_PROMPT))
        .collect();
    assert!(
        !third_hits.is_empty(),
        "the third prompt must reach the LLM body, saw: {second_req:?}"
    );
    let res_user: Vec<&String> = second_req
        .iter()
        .filter(|m| m.contains(RESUME_USER))
        .collect();
    assert_eq!(
        res_user.len(),
        2,
        "the live context carries the replayed user row + its assistant reply \
         exactly once each (a re-replay would double both); saw: {second_req:?}"
    );
}

/// Send `worker.prompt` with a `session_id` + a fresh run id, assert the
/// ack.  The mock LLM replies with a single clean text turn so the loop
/// terminates without tools.
fn prompt_with_session(client: &DaemonClient, message: &str) {
    let resp = client
        .call_raw(
            Command::WorkerPrompt,
            json!({
                "message": message,
                "session_id": "s-b2a",
                "run_id": format!("run-{}", std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0)),
            }),
        )
        .expect("prompt via daemon");
    assert!(resp.success, "prompt failed: {:?}", resp.error);
}
