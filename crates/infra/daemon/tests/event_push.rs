//! R2 3.4 — end-to-end event push.
//!
//! Starts a real daemon on a temp Unix socket with a mock `omp --mode rpc`
//! (python3), opens two `event.subscribe("worker")` connections, runs a
//! prompt through the daemon, and asserts both subscribers receive the full
//! `agent_start … agent_end` stream as `EventFrame` pushes.  Dropping one
//! subscriber must not disturb the other.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

use daemon::protocol::Command;
use daemon::{Daemon, DaemonClient, DaemonConfig, Subscription};
use serde_json::{Value, json};

const EVENT_TIMEOUT: Duration = Duration::from_secs(5);

/// Full turn stream the mock emits after each `prompt`, in wire order.
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

/// Drain `want` worker events from the subscription, returning the
/// `WorkerEvent` tags in arrival order.
fn collect_events(sub: &mut Subscription, want: usize) -> Vec<String> {
    let mut got = Vec::new();
    while got.len() < want {
        match sub
            .next_event(EVENT_TIMEOUT)
            .expect("subscription readable")
        {
            Some(frame) => {
                assert_eq!(frame.topic, "worker", "pushed on the subscribed topic");
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

fn prompt(client: &DaemonClient, message: &str) {
    let resp = client
        .call_raw(Command::WorkerPrompt, json!({ "message": message }))
        .expect("prompt via daemon");
    assert!(resp.success, "prompt failed: {:?}", resp.error);
}

#[test]
fn event_subscribe_pushes_full_turn_to_every_subscriber() {
    let dir = tempfile::tempdir().unwrap();
    let omp = mock_omp(dir.path());
    let socket = dir.path().join("daemon.sock");
    let db = dir.path().join("sessions.db");
    let mut daemon = Daemon::start(DaemonConfig {
        socket_path: Some(socket.clone()),
        omp_path: omp.to_string_lossy().into_owned(),
        session_db_path: Some(db),
    })
    .expect("daemon start");

    let client = DaemonClient::connect_to(&socket);
    assert!(client.ping().unwrap(), "daemon answers");

    // Subscribe first: the pump must be live before the run emits.
    let mut sub_a = client.subscribe("worker").expect("subscribe A");
    let mut sub_b = client.subscribe("worker").expect("subscribe B");

    prompt(&client, "go");
    let a = collect_events(&mut sub_a, EXPECTED.len());
    let b = collect_events(&mut sub_b, EXPECTED.len());
    assert_eq!(
        a, EXPECTED,
        "subscriber A saw the full agent_start..agent_end run"
    );
    assert_eq!(b, EXPECTED, "subscriber B saw the same run independently");

    // Disconnect A; B must keep streaming a second run untouched.
    drop(sub_a);
    prompt(&client, "go again");
    let b2 = collect_events(&mut sub_b, EXPECTED.len());
    assert_eq!(
        b2, EXPECTED,
        "dropping a subscriber does not disturb the rest"
    );

    // Explicit unsubscribe detaches exactly the given subscription.
    let sub_c = client.subscribe("worker").expect("subscribe C");
    assert!(
        client.event_unsubscribe(sub_c.id()).unwrap(),
        "first unsubscribe removes it"
    );
    assert!(
        !client.event_unsubscribe(sub_c.id()).unwrap(),
        "unsubscribing twice reports not-found"
    );

    daemon.shutdown();
}
