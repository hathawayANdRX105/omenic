//! G7-B — per-run attribution of the daemon's event stream.
//!
//! Two halves of the same fix:
//!
//! 1. `event_frame_carries_run_id` — a real daemon on a temp socket with a
//!    mock `omp --mode rpc` backend.  A prompt carrying `session_id` /
//!    `run_id` must produce `EventFrame` pushes stamped with that `run_id`,
//!    and the attribution must follow the prompt: a second prompt under a
//!    different run gets the second run's id, not a stale one.  That is the
//!    daemon side of "concurrent turns no longer write into whichever
//!    session sent most recently".
//! 2. `events_from_other_run_are_filtered` — the pure routing predicate the
//!    web subscriber uses (`EventFrame::belongs_to_run`, wrapped by
//!    `omenic_web-client`'s `RunFilteredSubscription`): another run's frames
//!    are dropped, unattributed frames still pass through.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

use daemon::protocol::Command;
use daemon::{Daemon, DaemonClient, DaemonConfig, EventFrame, Subscription};
use serde_json::{Value, json};

const EVENT_TIMEOUT: Duration = Duration::from_secs(5);

/// Full turn the mock emits after each prompt, in wire order.
const EXPECTED: [&str; 5] = [
    "agent_start",
    "message",
    "tool_execution_start",
    "tool_execution_end",
    "agent_end",
];

fn mock_omp(dir: &std::path::Path) -> PathBuf {
    let script = r#"#!/usr/bin/env python3
import json, sys

def out(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()

out({"type": "ready", "protocolVersion": 2,
     "supportedProtocolVersions": [2],
     "maxFrameBytes": 65536, "maxReassembledFrameBytes": 262144})

for line in sys.stdin:
    try:
        req = json.loads(line)
    except json.JSONDecodeError:
        continue
    t = req.get("type")
    out({"type": "response", "id": req.get("id"), "success": True})
    if t == "prompt":
        for ev in [
            {"type": "agent_start"},
            {"type": "message_update", "text": "hello"},
            {"type": "tool_execution_start", "toolName": "read", "input": {"path": "x"}},
            {"type": "tool_execution_end", "toolName": "read", "result": {"ok": True}},
            {"type": "agent_end"},
        ]:
            out(ev)
"#;
    let path = dir.join("mock-omp");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(script.as_bytes()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// Drain `want` pushed frames, asserting each is stamped with `run_id` and
/// arrives on the worker topic.  Returns the wire event tags in arrival order.
fn collect_run_events(sub: &mut Subscription, want: usize, run_id: &str) -> Vec<String> {
    let mut got = Vec::new();
    while got.len() < want {
        match sub
            .next_event(EVENT_TIMEOUT)
            .expect("subscription readable")
        {
            Some(frame) => {
                assert_eq!(frame.topic, "worker", "pushed on the subscribed topic");
                assert_eq!(
                    frame.run_id.as_deref(),
                    Some(run_id),
                    "frame attributed to the prompt's run, not a stale one",
                );
                let tag = frame
                    .event
                    .get("event")
                    .and_then(Value::as_str)
                    .unwrap_or("<missing>")
                    .to_string();
                got.push(tag);
            }
            None => panic!("timed out after {got:?}, expected {want} events"),
        }
    }
    got
}

/// `worker.prompt` with run attribution (WP-C shape: the client sends
/// `session_id` + `run_id`; the daemon records the run and, since G7-B,
/// stamps it onto the turn's pushed events).
fn prompt_run(client: &DaemonClient, session_id: &str, run_id: &str, message: &str) {
    let resp = client
        .call_raw(
            Command::WorkerPrompt,
            json!({
                "message": message,
                "session_id": session_id,
                "run_id": run_id,
            }),
        )
        .expect("prompt via daemon");
    assert!(resp.success, "prompt failed: {:?}", resp.error);
}

#[test]
fn event_frame_carries_run_id() {
    let dir = tempfile::tempdir().unwrap();
    let omp = mock_omp(dir.path());
    let socket = dir.path().join("daemon.sock");
    let db = dir.path().join("sessions.db");
    let mut daemon = Daemon::start(DaemonConfig {
        socket_path: Some(socket.clone()),
        omp_path: omp.to_string_lossy().into_owned(),
        session_db_path: Some(db),
        orbit_model: None,
        // Scope instruction discovery to the temp dir so the test never
        // picks up a real AGENTS.md from the repo it runs in.
        cwd: dir.path().to_path_buf(),
        max_turns: 64,
        mcp_servers: Vec::new(),
    })
    .expect("daemon start");

    let client = DaemonClient::connect_to(&socket);
    assert!(client.ping().unwrap(), "daemon answers");

    // Subscribe before the first prompt: `event.subscribe("worker")` is what
    // lazily starts the event pump, and it must be live before the run emits.
    let mut sub = client.subscribe("worker").expect("subscribe worker");

    prompt_run(&client, "s-g7", "r-first", "go");
    let first = collect_run_events(&mut sub, EXPECTED.len(), "r-first");
    assert_eq!(
        first, EXPECTED,
        "first run's full agent_start..agent_end stream arrived, attributed"
    );

    // A second turn under a different run must carry the *second* run's id.
    // This is the concurrency case the feature targets: without attribution,
    // both turns' events land on whichever session the subscriber is showing.
    prompt_run(&client, "s-g7", "r-second", "go again");
    let second = collect_run_events(&mut sub, EXPECTED.len(), "r-second");
    assert_eq!(
        second, EXPECTED,
        "second run's events are attributed to the second run, not the first"
    );

    daemon.shutdown();
}

#[test]
fn events_from_other_run_are_filtered() {
    // Frame attributed to the run the subscriber is showing.
    let ours = EventFrame::new("worker", json!({ "event": "message" })).with_run_id("r-active");
    // Frame from a concurrent turn the UI is not displaying.
    let theirs =
        EventFrame::new("worker", json!({ "event": "message" })).with_run_id("r-background");
    // Unattributed frame: pushed by an older daemon, or emitted outside any
    // attributed prompt.  A newer subscriber must not drop these.
    let legacy = EventFrame::new("worker", json!({ "event": "agent_start" }));

    assert!(
        ours.belongs_to_run("r-active"),
        "the active run's own events pass"
    );
    assert!(
        !theirs.belongs_to_run("r-active"),
        "another run's events are filtered out instead of mixing into the view"
    );
    assert!(
        !ours.belongs_to_run("r-something-else"),
        "the active run's events do not leak into another run's view"
    );
    assert!(
        legacy.belongs_to_run("r-active"),
        "unattributed frames pass through, not silently dropped"
    );
}
