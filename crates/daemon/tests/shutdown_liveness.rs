//! Regression: `daemon stop` must not hang behind an in-flight prompt.
//!
//! omp-mode `prompt` holds the worker mutex for its entire turn, and the
//! connection read loop takes that mutex per request. Serving
//! `Command::Shutdown` after the lock made `oi daemon stop` (whose client
//! reads with no deadline) wait for the running turn — observed as a stop
//! command frozen for 30s+. The server now answers `Shutdown` before the
//! lock and the client bounds its wait; this test proves the ack arrives
//! quickly while the lock is demonstrably held (the mock touches a marker
//! file from inside its `prompt` branch before sleeping past the assertion
//! window).

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use daemon::protocol::Command;
use daemon::{Daemon, DaemonClient, DaemonConfig};
use serde_json::json;

/// Mock `omp` that touches `<script>.locked` and stalls for 12s when a
/// prompt arrives — long enough that any request serialized behind the
/// worker lock would blow the assertion window below.
fn mock_omp(dir: &std::path::Path) -> PathBuf {
    let script = r#"#!/usr/bin/env python3
import json, sys, time

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
    if t == "prompt":
        open(sys.argv[0] + ".locked", "w").close()
        time.sleep(12)
    out({"type": "response", "id": req.get("id"), "success": True})
"#;
    let path = dir.join("mock-omp");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(script.as_bytes()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[test]
fn shutdown_answers_while_prompt_holds_the_worker_lock() {
    let dir = tempfile::tempdir().unwrap();
    let omp = mock_omp(dir.path());
    let marker = PathBuf::from(format!("{}.locked", omp.display()));
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
        ..Default::default()
    })
    .expect("daemon start");

    // Occupy the worker lock with a prompt the mock deliberately stalls.
    let busy = DaemonClient::connect_to(&socket);
    std::thread::spawn(move || {
        let _ = busy.call_raw(Command::WorkerPrompt, json!({ "message": "hold the lock" }));
    });

    // The marker proves the mock's prompt branch is running, i.e. the
    // dispatch (and its worker guard) is inside `w.prompt` right now.
    let deadline = Instant::now() + Duration::from_secs(8);
    while !marker.exists() {
        assert!(
            Instant::now() < deadline,
            "mock never entered the prompt branch — worker lock not provably held"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    // `shutdown` must be acked in milliseconds, not after the 12s stall.
    let client = DaemonClient::connect_to(&socket);
    let t0 = Instant::now();
    client.shutdown().expect("shutdown acked under load");
    let elapsed = t0.elapsed();
    assert!(
        elapsed < Duration::from_secs(4),
        "shutdown took {elapsed:?} — it queued behind the worker lock"
    );

    daemon.shutdown();
}
