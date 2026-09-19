//! Subagent seam: the daemon registers an in-process fork provider into the
//! container's `SubagentRuntimeService` (fiber key `harness.subagents`), so the
//! model-facing `subagent` tool resolves a provider in the daemon.
//!
//! These tests reproduce the registration block from `server.rs` (same
//! service type, same key, same `ForkProvider` construction) plus the
//! read-only tool filter, and assert:
//! - register → get returns `Some` and the provider identity round-trips
//!   (`name()` is `"fork"`);
//! - registering a second provider under the same name overwrites (the
//!   service's documented contract — composition controls load order);
//! - the read-only tool filter via `filter_builtin_tools` yields exactly
//!   the three fork tools.

use std::sync::Arc;

use orbit::LlmBackend;

use omenic_harness_subagent::{ForkProvider, SubagentRuntimeService};

/// The daemon's fork provider registration (server.rs block, condensed):
/// a service + a `ForkProvider` built over the read-only tool subset.
fn register_fork(service: &SubagentRuntimeService) {
    let model = adaptor::Model {
        api_key: "test".into(),
        model: "test-model".into(),
        base_url: None,
        max_tokens: None,
    };
    let backend: Arc<dyn LlmBackend + Send + Sync> = Arc::new(orbit::HttpLlm);
    let wanted = ["read_file", "grep", "glob"];
    let tools: Arc<Vec<Box<dyn tools::Tool>>> =
        Arc::new(omenic_harness_tools::filter_builtin_tools(&wanted));
    service.register(
        "fork",
        Arc::new(ForkProvider::new(backend, model, tools, 8)),
    );
}

/// After `register("fork", …)` the service hands back a provider that
/// identifies as `"fork"` — the seam is live, not a throwaway.
#[test]
fn fork_provider_registers_and_resolves() {
    let service = SubagentRuntimeService::default();
    assert!(
        service.get("fork").is_none(),
        "no provider before registration"
    );
    register_fork(&service);
    let provider = service.get("fork").expect("fork provider must resolve");
    assert_eq!(provider.name(), "fork");
}

/// Re-registering under the same name overwrites (idempotent-safe; the
/// documented composition-order contract — last writer wins).
#[test]
fn re_register_same_name_overwrites() {
    let service = SubagentRuntimeService::default();
    register_fork(&service);
    register_fork(&service);
    let provider = service.get("fork").expect("fork provider must resolve");
    assert_eq!(provider.name(), "fork");
    assert_eq!(service.providers(), vec!["fork".to_string()]);
}

/// The daemon's filter over `tools::builtin_tools()` yields exactly the
/// three read-only tool names — no write/exec tool leaks into a fork run.
#[test]
fn fork_tools_filter_yields_read_only_subset() {
    let wanted = ["read_file", "grep", "glob"];
    let names: Vec<String> = omenic_harness_tools::filter_builtin_tools(&wanted)
        .into_iter()
        .map(|t| tools::def(&*t).name)
        .collect();
    assert_eq!(
        names,
        vec![
            "read_file".to_string(),
            "grep".to_string(),
            "glob".to_string()
        ],
        "fork tool set is exactly the read-only built-in subset"
    );
}

// ────────────────────────────────────────────────────────────────────────
// Smoke: the full daemon path — `Daemon::start` → `assemble` → `orbit_setup`
// registers the fork provider into the container, and the model-facing
// `subagent` tool in the worker's catalog actually resolves it.
//
// A mock OpenAI server (g6_e2e pattern) replays one tool-call turn: the
// parent loop asks the model to delegate via the `subagent` tool, orbit
// dispatches it through the container catalog, the fork provider spawns
// its own runner, and the runner's HTTP call to the same mock completes
// with a text turn. The assertion is that the `subagent` tool's result
// carries `status: completed` — which only happens if the provider
// registration in `orbit_setup` is real.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use daemon::protocol::Command;
use daemon::{Daemon, DaemonClient, DaemonConfig, EventFrame, Subscription};
use serde_json::{Value, json};
use tempfile::tempdir;

const EVENT_TIMEOUT: Duration = Duration::from_secs(10);

/// A turn that calls the `subagent` tool once and finishes.
fn tool_call_turn() -> String {
    let call = json!({
        "choices": [{
            "delta": {
                "role": "assistant",
                "tool_calls": [{
                    "index": 0,
                    "id": "call_subagent_0",
                    "type": "function",
                    "function": {
                        "name": "subagent",
                        "arguments": "{\"prompt\":\"Say SUBAGENT-OK\"}"
                    }
                }]
            },
            "finish_reason": Value::Null
        }]
    });
    let finish = json!({
        "choices": [{ "delta": {}, "finish_reason": "stop" }]
    });
    format!("data: {}\n\ndata: {}\n\n", call, finish)
}

/// One text-delta turn that finishes — the subagent's completion reply.
fn one_text_turn(delta: &str) -> String {
    let chunk = json!({
        "choices": [{
            "delta": { "content": delta },
            "finish_reason": "stop"
        }]
    });
    format!("data: {}\n\n", chunk)
}

// The smoke's mock predates the tool-call path and answers every
// connection with a plain text turn, which cannot drive a loop that
// delegates. It is driven from a background thread that replies as fast as
// the client can ask, counting the requests it has served so the test can
// wait for call #N instead of guessing at a sleep long enough.
//
// The whole test is therefore bounded twice over: by the mock's deadline
// (a test that never gets the traffic it expects fails on its own) and by
// the test's own deadlines on the frame stream. Neither is a fixed sleep —
// both are "wait until the condition holds, or fail saying what never
// happened".
struct MockOpenAiSmoke {
    addr: String,
    bodies: Arc<std::sync::Mutex<Vec<String>>>,
    served: Arc<(Mutex<usize>, Condvar)>,
}

impl MockOpenAiSmoke {
    /// Start replying `2n+1` with the tool-call turn and `2n+2` with the
    /// text turn, so the parent loop asks to delegate on its first request
    /// and the fork's runner gets a completion on its own first request.
    ///
    /// The client opens a fresh connection per request, so the body counter
    /// — not the connection counter — is what advances the script.
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
        let addr = listener.local_addr().expect("local addr");
        let bodies = Arc::new(std::sync::Mutex::new(Vec::new()));
        let bodies_srv = Arc::clone(&bodies);
        let served = Arc::new((Mutex::new(0usize), Condvar::new()));
        let served_srv = Arc::clone(&served);

        thread::spawn(move || {
            listener
                .set_nonblocking(true)
                .expect("mock listener nonblocking");
            let deadline = Instant::now() + Duration::from_secs(60);
            while Instant::now() < deadline {
                let stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => break,
                };
                let bodies = Arc::clone(&bodies_srv);
                let served = Arc::clone(&served_srv);
                thread::spawn(move || {
                    // The script position is claimed on accept so that two
                    // connections arriving together still get distinct
                    // replies. The count the test waits on is *not* bumped
                    // here: `serve_smoke` publishes it once the body is read,
                    // which is what keeps `received()` from being shorter
                    // than the count `wait_for_served` reported.
                    let seq = {
                        static NEXT: AtomicUsize = AtomicUsize::new(0);
                        NEXT.fetch_add(1, Ordering::SeqCst) + 1
                    };
                    let reply = if seq % 2 == 1 {
                        tool_call_turn()
                    } else {
                        one_text_turn("SUBAGENT-OK")
                    };
                    serve_smoke(stream, bodies, served, reply);
                });
            }
        });

        MockOpenAiSmoke {
            addr: format!("http://127.0.0.1:{}", addr.port()),
            bodies,
            served,
        }
    }

    fn received(&self) -> Vec<String> {
        self.bodies.lock().expect("bodies").clone()
    }

    /// Block until at least `n` request *bodies* have been read off the
    /// wire. Returns `false` if the deadline passes first, so a test that
    /// sees too little traffic fails with its own message rather than
    /// panicking inside a helper — and, since the count only advances once
    /// a body is complete, `received()` is never shorter than `n` when this
    /// returns `true`.
    fn wait_for_served(&self, n: usize, timeout: Duration) -> bool {
        let (lock, cvar) = &*self.served;
        let deadline = Instant::now() + timeout;
        let mut count = lock.lock().expect("served count");
        while *count < n {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let (guard, _) = cvar
                .wait_timeout(count, deadline - now)
                .expect("served condvar");
            count = guard;
        }
        true
    }
}

fn serve_smoke(
    mut stream: TcpStream,
    bodies: Arc<std::sync::Mutex<Vec<String>>>,
    served: Arc<(Mutex<usize>, Condvar)>,
    reply: String,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    // Read until the header terminator. Reading one byte at a time is what
    // makes the split exact: the body begins at the byte after `\r\n\r\n`,
    // and a buffered read would have to hand any overshoot back.
    let mut head: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 1];
    while let Ok(n) = stream.read(&mut tmp) {
        if n == 0 {
            break;
        }
        head.push(tmp[0]);
        if head.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let head_str = String::from_utf8_lossy(&head).to_string();
    let body_len = head_str
        .split("\r\n")
        .find_map(|l| {
            l.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .and_then(|v| v.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);

    // `head` holds headers only, so the body starts from empty. Slicing it
    // from an offset derived from `body_len` (as this once did) seeds the
    // buffer with header bytes and then stops reading early — the request
    // reaches the test truncated, and a truncated tool list is
    // indistinguishable from a model that chose not to call one.
    let mut body_bytes: Vec<u8> = Vec::new();
    while body_bytes.len() < body_len {
        let mut chunk = [0u8; 4096];
        let n = match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        body_bytes.extend_from_slice(&chunk[..n]);
    }
    let body = String::from_utf8_lossy(&body_bytes[..body_len.min(body_bytes.len())]).to_string();
    bodies.lock().expect("bodies").push(body);
    // Publish only now: the body is complete and already in `bodies`, so a
    // waiter that sees the new count can read it without racing this thread.
    {
        let (lock, cvar) = &*served;
        *lock.lock().expect("served count") += 1;
        cvar.notify_all();
    }
    let mut wire = String::from("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n");
    wire.push_str(&reply);
    let _ = stream.write_all(wire.as_bytes());
    let _ = stream.flush();
}

fn daemon_cfg_smoke(
    dir: &std::path::Path,
    mock: &MockOpenAiSmoke,
    max_turns: usize,
) -> DaemonConfig {
    DaemonConfig {
        socket_path: Some(dir.join("daemon.sock")),
        omp_path: "omp".into(),
        session_db_path: Some(dir.join("sessions.db")),
        orbit_model: Some(adaptor::Model {
            api_key: "test-key".into(),
            model: "subagent-smoke".into(),
            base_url: Some(mock.addr.clone()),
            max_tokens: Some(1024),
        }),
        cwd: dir.to_path_buf(),
        max_turns,
        ..Default::default()
    }
}

/// Poll the frame stream until `pred` accepts a frame, or fail saying which
/// frame never arrived. The subscription's 200ms ticks keep this honest: it
/// returns as soon as the condition holds rather than after a fixed wait,
/// and a `None` (no frame ready yet) is a retry, not a reason to stop.
fn wait_for_frame(
    sub: &mut Subscription,
    what: &str,
    mut pred: impl FnMut(&EventFrame) -> bool,
) -> EventFrame {
    let deadline = Instant::now() + EVENT_TIMEOUT;
    let mut seen: Vec<String> = Vec::new();
    loop {
        if Instant::now() >= deadline {
            panic!("no {what} within {EVENT_TIMEOUT:?}; saw events: {seen:?}");
        }
        match sub
            .next_event(Duration::from_millis(200))
            .expect("subscription readable")
        {
            Some(frame) => {
                if pred(&frame) {
                    return frame;
                }
                seen.push(
                    frame
                        .event
                        .get("event")
                        .and_then(Value::as_str)
                        .unwrap_or("<missing>")
                        .to_string(),
                );
            }
            None => continue,
        }
    }
}

/// Smoke: `Daemon::start` → `assemble` → `orbit_setup` → worker catalog
/// contains `subagent` → model delegates → fork provider runs → completed.
#[test]
fn daemon_subagent_seam_e2e() {
    let dir = tempdir().expect("temp dir");
    std::fs::write(dir.path().join("AGENTS.md"), "# bare\n").expect("AGENTS.md");

    // Odd request: the parent loop's delegation turn. Even request: the
    // fork runner's plain text completion.
    let mock = MockOpenAiSmoke::start();
    let cfg = daemon_cfg_smoke(dir.path(), &mock, 8);
    let mut daemon = Daemon::start(cfg).expect("daemon start");

    let client = DaemonClient::connect_to(daemon.socket_addr().path());
    let mut sub = client.subscribe("worker").expect("subscribe");

    // Wait for the mock to have served the *parent* request before reading
    // frames. The pre-fix smoke answered every connection with the tool-call
    // turn, so it produced no traffic at all: the model's delegation turn was
    // streamed back to it as the answer to its own delegation, and the loop
    // finished without ever reaching the fork. Stating the precondition
    // explicitly is what keeps an early exit from being reported as a
    // missing frame.
    //
    // This must come *after* the prompt, not before: in orbit mode
    // `WorkerPrompt` only enqueues the message and acks, so the engine thread
    // sends the LLM request strictly after `call_raw` returns and no request
    // can be in flight while the client is still composing one.
    let resp = client
        .call_raw(Command::WorkerPrompt, json!({ "message": "delegate" }))
        .expect("prompt via daemon");
    assert!(resp.success, "prompt failed: {:?}", resp.error);

    assert!(
        mock.wait_for_served(1, EVENT_TIMEOUT),
        "the daemon never sent the parent loop's first LLM request"
    );

    // The mock must have served 2 requests: parent + subagent runner. The
    // runner's request is issued while the parent's delegate turn is being
    // handled, i.e. before that turn can complete, so waiting here makes the
    // second precondition explicit rather than implied.
    assert!(
        mock.wait_for_served(2, EVENT_TIMEOUT),
        "expected parent + subagent HTTP calls, mock only served {}",
        mock.received().len()
    );

    // Find the `subagent` tool execution end. WorkerEvent serializes as
    // `{"event":"tool_execution_end","name":"subagent","result":{...}}`
    // (serde tag="event"), so the frame's `event` field carries it. Waiting
    // on the tools' end instead of on `agent_end` keeps the assertion about
    // the subagent, not about the run's shutdown tail.

    let subagent_end = wait_for_frame(&mut sub, "tool_execution_end for `subagent`", |f| {
        f.event.get("event").and_then(Value::as_str) == Some("tool_execution_end")
            && f.event.get("name").and_then(Value::as_str) == Some("subagent")
    });
    let result = subagent_end
        .event
        .get("result")
        .expect("tool_execution_end carries its result");
    let result_str = result.to_string();
    assert!(
        result_str.contains("completed"),
        "subagent tool must report completed, got: {result_str}"
    );

    drop(client);
    daemon.shutdown();
}
