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

use std::path::Path;

use daemon::{Daemon, DaemonClient};
use serde_json::{Value, json};
use tempfile::tempdir;

use common::{MockOpenAi, daemon_cfg, drain_events, one_text_turn, prompt};

mod common;

/// Distinctive token written into the workspace `AGENTS.md`. If instruction
/// injection works end to end, this string is inside the HTTP request body
/// the mock server received.
const MARKER: &str = "G6-E2E-INSTRUCTION-MARKER";

fn workspace_with_agents_md(dir: &Path) {
    std::fs::write(
        dir.join("AGENTS.md"),
        format!("# Test workspace\n\nRule: emit {MARKER} in every reply.\n"),
    )
    .expect("write AGENTS.md");
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
                    "finish_reason": Value::Null
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
