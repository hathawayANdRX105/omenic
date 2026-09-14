//! R2 2.3 — push-mode `Worker::subscribe`.
//!
//! A mock `omp --mode rpc` (python3) answers the handshake/negotiation and,
//! on `prompt`, emits a response frame followed by the agent event stream.
//! The tests assert the pump forwards the full event sequence to the
//! subscriber while `prompt()` still returns its own response.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

use rpc::worker::{Worker, WorkerEvent};

/// Writes the mock omp script into `dir` and returns its path.
pub fn mock_omp(dir: &std::path::Path) -> PathBuf {
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

/// Receives one event, failing the test if none arrives in 2s.
fn recv(rx: &std::sync::mpsc::Receiver<WorkerEvent>) -> WorkerEvent {
    rx.recv_timeout(Duration::from_secs(2))
        .expect("event pump delivered nothing")
}

fn event_kind(event: &WorkerEvent) -> &'static str {
    match event {
        WorkerEvent::AgentStart => "agent_start",
        WorkerEvent::Message { .. } => "message",
        WorkerEvent::ToolExecutionStart { .. } => "tool_execution_start",
        WorkerEvent::ToolExecutionEnd { .. } => "tool_execution_end",
        WorkerEvent::AgentEnd => "agent_end",
        WorkerEvent::Error { .. } => "error",
        WorkerEvent::Unknown(_) => "unknown",
    }
}

#[test]
fn subscribe_receives_prompt_event_sequence() {
    let dir = tempfile::tempdir().unwrap();
    let omp = mock_omp(dir.path());
    let mut worker = Worker::new(omp.to_str().unwrap()).expect("spawn mock omp");

    let rx = worker.subscribe("worker");
    let resp = worker.prompt("hi").expect("prompt through pump");
    assert_eq!(resp.get("success").and_then(|v| v.as_bool()), Some(true));

    let kinds: Vec<&str> = (0..5).map(|_| event_kind(&recv(&rx))).collect();
    assert_eq!(
        kinds,
        [
            "agent_start",
            "message",
            "tool_execution_start",
            "tool_execution_end",
            "agent_end",
        ]
    );
}

#[test]
fn dropped_receiver_is_unregistered_other_receiver_keeps_flowing() {
    let dir = tempfile::tempdir().unwrap();
    let omp = mock_omp(dir.path());
    let mut worker = Worker::new(omp.to_str().unwrap()).expect("spawn mock omp");

    let dead = worker.subscribe("worker");
    let alive = worker.subscribe("worker");
    drop(dead); // receiver gone before any event -> pump must survive it

    worker.prompt("hi").expect("prompt");
    let kinds: Vec<&str> = (0..5).map(|_| event_kind(&recv(&alive))).collect();
    assert_eq!(kinds.len(), 5, "tail of the stream kept flowing");
    assert_eq!(kinds[0], "agent_start");
    assert_eq!(kinds[4], "agent_end");
    // Pull mode is closed while the pump runs.
    assert!(worker.read_event().is_err());
}
