//! End-to-end CLI tests for daemon lifecycle and session attach/resume.
//!
//! Same pattern as `session_cli.rs`: an in-process daemon on a tempdir socket,
//! the real `oi` binary as a subprocess, no cloud API keys involved. The
//! worker-spawn failure path (missing omp binary) is exercised deliberately:
//! `session resume` must surface a structured error AND leave a `spawn_failed`
//! record in the run ledger — that is the main-flow coverage #310 asks for.

use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use daemon::{Daemon, DaemonConfig};
use session::SessionRole;
use tempfile::TempDir;

fn start_daemon(dir: &Path, tag: &str) -> Daemon {
    let cfg = DaemonConfig {
        socket_path: Some(dir.join(format!("{tag}.sock"))),
        omp_path: "omp-not-installed-for-cli-tests".to_string(),
        session_db_path: Some(dir.join(format!("{tag}.db"))),
    };
    Daemon::start(cfg).expect("start daemon")
}

fn wait_for_socket(path: &Path) {
    for _ in 0..500 {
        if UnixStream::connect(path).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("daemon socket never came up: {path:?}");
}

fn run_oi(
    data_dir: &Path,
    socket: &Path,
    db: &Path,
    args: &[&str],
    extra: &[(&str, &str)],
) -> (String, String, bool) {
    let exe = env!("CARGO_BIN_EXE_oi");
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .env("OMENIC_DATA_DIR", data_dir)
        .env("OMENIC_DAEMON_SOCKET", socket)
        .env("OMENIC_SESSION_DB", db)
        .env("OMENIC_OMP_PATH", "omp-not-installed-for-cli-tests");
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let output = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn oi");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    )
}

#[test]
fn daemon_start_reports_already_running() {
    let dir = TempDir::new().unwrap();
    let sock = dir.path().join("start.sock");
    let db = dir.path().join("start.db");
    let _daemon = start_daemon(dir.path(), "start");
    wait_for_socket(&sock);

    let (stdout, stderr, ok) = run_oi(
        dir.path(),
        &sock,
        &db,
        &["daemon", "start"],
        &[("OMENIC_DAEMON_PATH", "/nonexistent-must-not-be-used")],
    );
    assert!(ok, "stderr: {stderr}");
    assert!(stdout.contains("already running"), "stdout: {stdout}");
}

#[test]
fn daemon_start_errors_when_binary_missing() {
    let dir = TempDir::new().unwrap();
    let sock = dir.path().join("nostart.sock");
    let db = dir.path().join("nostart.db");
    // No daemon listening: start must fall through to binary lookup and fail
    // with an actionable message (never a hang).
    let (_stdout, stderr, ok) = run_oi(
        dir.path(),
        &sock,
        &db,
        &["daemon", "start"],
        &[("OMENIC_DAEMON_PATH", "/nonexistent/daemon")],
    );
    assert!(!ok);
    assert!(
        stderr.contains("daemon binary not found"),
        "stderr: {stderr}"
    );
}

#[test]
fn session_attach_shows_messages() {
    let dir = TempDir::new().unwrap();
    let sock = dir.path().join("attach.sock");
    let db = dir.path().join("attach.db");
    let _daemon = start_daemon(dir.path(), "attach");
    wait_for_socket(&sock);

    let client = daemon::DaemonClient::connect_to(&sock);
    client.session_create("alpha", "first").unwrap();
    client
        .session_append("alpha", SessionRole::User, "hello daemon")
        .unwrap();
    client
        .session_append("alpha", SessionRole::Assistant, "hi back")
        .unwrap();

    let (stdout, stderr, ok) = run_oi(
        dir.path(),
        &sock,
        &db,
        &["--json", "session", "attach", "alpha"],
        &[],
    );
    assert!(ok, "stderr: {stderr}");
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("parse JSON");
    assert_eq!(v["session"]["id"], "alpha");
    let msgs = v["messages"].as_array().expect("messages array");
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0]["text"], "hello daemon");
}

#[test]
fn session_attach_missing_session_errors() {
    let dir = TempDir::new().unwrap();
    let sock = dir.path().join("attach404.sock");
    let db = dir.path().join("attach404.db");
    let _daemon = start_daemon(dir.path(), "attach404");
    wait_for_socket(&sock);

    let (_stdout, stderr, ok) =
        run_oi(dir.path(), &sock, &db, &["session", "attach", "ghost"], &[]);
    assert!(!ok);
    assert!(stderr.contains("session not found"), "stderr: {stderr}");
}

#[test]
fn session_resume_records_failed_run_in_ledger() {
    let dir = TempDir::new().unwrap();
    let sock = dir.path().join("resume.sock");
    let db = dir.path().join("resume.db");
    let _daemon = start_daemon(dir.path(), "resume");
    wait_for_socket(&sock);

    let client = daemon::DaemonClient::connect_to(&sock);
    client.session_create("alpha", "first").unwrap();

    // Worker cannot spawn (fake omp path): resume must fail cleanly...
    let (_stdout, stderr, ok) = run_oi(
        dir.path(),
        &sock,
        &db,
        &["session", "resume", "alpha", "continue please"],
        &[],
    );
    assert!(!ok, "resume should fail without a real worker");
    assert!(
        stderr.contains("daemon error") || stderr.contains("worker"),
        "stderr: {stderr}"
    );

    // ...yet the ledger keeps the correlated run record for reconnects.
    let runs = client.run_list(50).unwrap();
    let rec = runs
        .iter()
        .find(|r| r.session_id == "alpha")
        .expect("run record for session alpha");
    assert!(rec.run_id.starts_with("r-"));
    assert_eq!(rec.status.as_deref(), Some("spawn_failed"));
    assert!(rec.finished_at_ms.is_some());
}
