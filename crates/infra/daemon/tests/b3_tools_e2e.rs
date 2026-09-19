//! B3 smoke — the jobs and terminal tools behind the real daemon path.
//!
//! `jobs_terminal_test.rs` proves the tool structs work against the real
//! registries. It does not prove the daemon wires them up: that needs
//! `Daemon::start` -> `orbit_setup` -> `combined_tools` -> the loop's tool
//! dispatch, all real, with the request leaving the process.
//!
//! These tests close that gap. A daemon runs on a temp socket against a mock
//! OpenAI server whose replies are tool calls; the assertion is on the bytes
//! the mock received *back* in the following request — the tool's own output,
//! which only a working end-to-end dispatch can put there. The terminal case
//! spans four turns so the pty session is created, written to and read from
//! by separate tool calls: that cross-turn survival is what the registry
//! exists to provide.

use daemon::{Daemon, DaemonClient};
use serde_json::{Value, json};
use tempfile::tempdir;

use common::{MockOpenAi, daemon_cfg, drain_events, one_text_turn, prompt};

mod common;

/// One assistant round that issues a single tool call and finishes the turn.
/// The arguments are sent as a JSON *string*, matching what a real model
/// streams: the loop parses them per-call.
fn tool_turn(name: &str, args: &Value) -> String {
    let call = json!({
        "choices": [{
            "delta": {
                "role": "assistant",
                "tool_calls": [{
                    "index": 0,
                    "id": "call_b3",
                    "type": "function",
                    "function": { "name": name, "arguments": args.to_string() }
                }]
            },
            "finish_reason": Value::Null
        }]
    });
    let finish = json!({
        "choices": [{ "delta": {}, "finish_reason": "tool_calls" }]
    });
    format!("data: {}\ndata: {}\n\n", call, finish)
}

const JOB_MARKER: &str = "B3-JOB-SMOKE";
const TERM_MARKER: &str = "B3-TERM-SMOKE";

/// Run a daemon on `dir` against `replies` and return every HTTP body the
/// mock saw, in arrival order — body *n+1* carries round *n*'s tool results.
fn run_turns(dir: &std::path::Path, replies: Vec<String>, max_turns: usize) -> Vec<String> {
    std::fs::write(dir.join("AGENTS.md"), "# smoke\n").expect("AGENTS.md");
    let mock = MockOpenAi::start(replies);
    let cfg = daemon_cfg(dir, &mock, max_turns);
    let mut daemon = Daemon::start(cfg).expect("daemon start");

    let client = DaemonClient::connect_to(daemon.socket_addr().path());
    let mut sub = client.subscribe("worker").expect("subscribe");
    prompt(&client, "go");
    drain_events(&mut sub);
    drop(client);
    daemon.shutdown();

    mock.received()
}

/// `jobs_start` through the real daemon: the tool's own reply must reach the
/// next request body the loop sends.
#[test]
fn jobs_tool_runs_behind_the_real_daemon() {
    let dir = tempdir().expect("temp dir");
    let bodies = run_turns(
        dir.path(),
        vec![
            tool_turn(
                "jobs_start",
                &json!({"command": format!("echo {JOB_MARKER}")}),
            ),
            one_text_turn("done"),
        ],
        4,
    );
    assert!(
        bodies.len() >= 2,
        "expected a follow-up request after the tool call; got {}",
        bodies.len()
    );
    assert!(
        bodies[1].contains("started job"),
        "the jobs_start reply never reached the next request: {}",
        bodies[1]
    );
}

/// A pty session lives across turns: create, write, read in three separate
/// tool calls. The read's output reaches the fourth request body, which only
/// happens if the session survived the turn boundary and the reader thread
/// delivered the bytes.
#[test]
fn terminal_session_survives_across_turns() {
    let dir = tempdir().expect("temp dir");
    let bodies = run_turns(
        dir.path(),
        vec![
            tool_turn("terminal_create", &json!({"shell": "sh"})),
            tool_turn(
                "terminal_write",
                &json!({"id": "term-1", "data": format!("echo {TERM_MARKER}\n")}),
            ),
            tool_turn(
                "terminal_read",
                &json!({"id": "term-1", "timeout_ms": 2_000}),
            ),
            one_text_turn("done"),
        ],
        8,
    );
    assert!(
        bodies.len() >= 4,
        "expected 4 requests, got {}",
        bodies.len()
    );
    assert!(
        bodies[1].contains("terminal term-1 opened"),
        "create reply missing: {}",
        bodies[1]
    );
    assert!(
        bodies[3].contains(TERM_MARKER),
        "terminal_read output never reached the loop: {}",
        bodies[3]
    );
}
