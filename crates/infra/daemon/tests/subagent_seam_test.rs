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
use std::thread;
use std::time::Duration;

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

struct MockOpenAiSmoke {
    addr: String,
    bodies: Arc<std::sync::Mutex<Vec<String>>>,
}

impl MockOpenAiSmoke {
    /// `first_call_replies` applies to connection 1 (the parent loop),
    /// `later_replies` to every subsequent connection.
    fn start(first_call_replies: Vec<String>, later_replies: Vec<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
        let addr = listener.local_addr().expect("local addr");
        let bodies = Arc::new(std::sync::Mutex::new(Vec::new()));
        let bodies_srv = Arc::clone(&bodies);
        let first_srv = first_call_replies.clone();
        let later_srv = later_replies.clone();
        let conn = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let bodies = Arc::clone(&bodies_srv);
                let first = first_srv.clone();
                let later = later_srv.clone();
                let conn = Arc::clone(&conn);
                thread::spawn(move || {
                    let n = conn.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let replies: Vec<String> = if n == 0 {
                        first.iter().chain(&later).cloned().collect()
                    } else {
                        later.iter().cloned().collect()
                    };
                    serve_smoke(stream, bodies, replies);
                });
            }
        });

        MockOpenAiSmoke {
            addr: format!("http://127.0.0.1:{}", addr.port()),
            bodies,
        }
    }

    fn received(&self) -> Vec<String> {
        self.bodies.lock().expect("bodies").clone()
    }
}

fn serve_smoke(
    mut stream: TcpStream,
    bodies: Arc<std::sync::Mutex<Vec<String>>>,
    replies: Vec<String>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf: Vec<u8> = Vec::new();
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
    let mut rest = buf[buf.len() - body_len.min(buf.len())..].to_vec();
    while rest.len() < body_len {
        let mut chunk = [0u8; 512];
        let n = stream.read(&mut chunk).unwrap_or(0);
        if n == 0 {
            break;
        }
        rest.extend_from_slice(&chunk[..n]);
    }
    let body = String::from_utf8_lossy(&rest[..body_len.min(rest.len())]).to_string();
    bodies.lock().expect("bodies").push(body);
    let mut reply = String::from("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n");
    for r in &replies {
        reply.push_str(r);
    }
    let _ = stream.write_all(reply.as_bytes());
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
        mcp_servers: Vec::new(),
        llm_fallbacks: Vec::new(),
    }
}

fn drain_events_smoke(sub: &mut Subscription) -> Vec<EventFrame> {
    let mut out = Vec::new();
    loop {
        match sub
            .next_event(EVENT_TIMEOUT)
            .expect("subscription readable")
        {
            Some(frame) => out.push(frame),
            None => return out,
        }
    }
}

/// Smoke: `Daemon::start` → `assemble` → `orbit_setup` → worker catalog
/// contains `subagent` → model delegates → fork provider runs → completed.
#[test]
fn daemon_subagent_seam_e2e() {
    let dir = tempdir().expect("temp dir");
    std::fs::write(dir.path().join("AGENTS.md"), "# bare\n").expect("AGENTS.md");

    // Parent loop: one turn that calls the `subagent` tool, then finishes.
    // Subagent: a plain text reply (connection 2+).
    let mock = MockOpenAiSmoke::start(vec![tool_call_turn()], vec![one_text_turn("SUBAGENT-OK")]);
    let cfg = daemon_cfg_smoke(dir.path(), &mock, 8);
    let mut daemon = Daemon::start(cfg).expect("daemon start");

    let client = DaemonClient::connect_to(daemon.socket_addr().path());
    let mut sub = client.subscribe("worker").expect("subscribe");
    let resp = client
        .call_raw(Command::WorkerPrompt, json!({ "message": "delegate" }))
        .expect("prompt via daemon");
    assert!(resp.success, "prompt failed: {:?}", resp.error);
    let events = drain_events_smoke(&mut sub);

    // Find the `subagent` tool execution end. WorkerEvent serializes as
    // `{"event":"tool_execution_end","name":"subagent","result":{...}}`
    // (serde tag="event"), so the frame's `event` field carries it.
    let subagent_end = events
        .iter()
        .find(|f| {
            f.event.get("event").and_then(Value::as_str) == Some("tool_execution_end")
                && f.event.get("name").and_then(Value::as_str) == Some("subagent")
        })
        .expect("a tool_execution_end frame for the subagent tool");
    let result = subagent_end
        .event
        .get("result")
        .expect("tool_execution_end carries its result");
    let result_str = result.to_string();
    assert!(
        result_str.contains("completed"),
        "subagent tool must report completed, got: {result_str}"
    );

    // The mock must have received at least 2 calls: parent + subagent.
    let bodies = mock.received();
    assert!(
        bodies.len() >= 2,
        "expected parent + subagent HTTP calls, got {}",
        bodies.len()
    );

    drop(client);
    daemon.shutdown();
}
