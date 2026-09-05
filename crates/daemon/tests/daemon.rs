//! Integration tests for the daemon crate.
//!
//! Covers the full lifecycle against a real Unix-domain socket:
//!
//! * start → connect → `daemon.ping` round-trip → session CRUD → shutdown
//! * single-instance enforcement (second `start` reports `AlreadyRunning`)
//! * socket + lock + pid artifacts removed on drop
//! * disconnect + reconnect keeps state intact
//! * session delete removes the row from the on-disk libSQL DB
//!
//! We never spawn a real `omp` worker — the binary at `OMENIC_OMP_PATH`
//! defaults to a path that doesn't exist, but every code path that touches
//! the worker is guarded by a `worker.*` command and the daemon-level
//! commands (`daemon.*`, `session.*`) work fine without it.

use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use daemon::{Command, Daemon, DaemonConfig, Request, Response, ResponseError};
use parking_lot::Mutex;
use serde_json::{Value, json};
use session::{MAX_LIMIT, SessionRole};
use tempfile::TempDir;

/// One env var mutation at a time.  Tests that touch env run under this lock
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Build a `DaemonConfig` rooted at `dir`.  No real `omp` — the daemon
/// itself doesn't need one until someone issues a `worker.*` request.
fn daemon_config_in(dir: &TempDir, tag: &str) -> DaemonConfig {
    DaemonConfig {
        socket_path: Some(dir.path().join(format!("{tag}.sock"))),
        omp_path: "omp-not-installed-for-tests".to_string(),
        session_db_path: Some(dir.path().join(format!("{tag}.db"))),
    }
}

/// Send one newline-delimited JSON request and read one response.
fn round_trip(stream: &mut UnixStream, req: &Request) -> Response {
    let payload = serde_json::to_string(req).expect("serialize");
    stream.write_all(payload.as_bytes()).expect("write");
    stream.write_all(b"\n").expect("write nl");

    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = stream.read(&mut byte).expect("read");
        if n == 0 {
            panic!("server closed before responding to {payload:?}");
        }
        if byte[0] == b'\n' {
            break;
        }
        buf.push(byte[0]);
    }
    serde_json::from_slice(&buf).expect("parse response")
}

fn connect(sock: &PathBuf) -> UnixStream {
    // The accept loop polls every 50ms, so the client may race briefly
    // against the server coming up.  Retry the connect a few times.
    let mut last_err = None;
    for _ in 0..50 {
        match UnixStream::connect(sock) {
            Ok(s) => return s,
            Err(e) => {
                last_err = Some(e);
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    panic!(
        "could not connect to daemon socket `{sock:?}`: {}",
        last_err.expect("err")
    );
}

#[test]
fn ping_round_trip_returns_pong() {
    let dir = TempDir::new().unwrap();
    let cfg = daemon_config_in(&dir, "ping");
    let daemon = Daemon::start(cfg).unwrap();

    let mut client = connect(&daemon.socket_addr().path().to_path_buf());
    let resp = round_trip(&mut client, &Request::new(Command::Ping).with_id("r1"));
    assert!(resp.success);
    assert_eq!(resp.id.as_deref(), Some("r1"));
    assert_eq!(resp.data, Some(json!({ "pong": true })));
}

#[test]
fn session_crud_round_trips_through_the_socket() {
    let dir = TempDir::new().unwrap();
    let cfg = daemon_config_in(&dir, "crud");
    let daemon = Daemon::start(cfg).unwrap();

    let mut client = connect(&daemon.socket_addr().path().to_path_buf());

    // Create
    let resp = round_trip(
        &mut client,
        &Request::new(Command::SessionCreate)
            .with_id("c1")
            .with_params(json!({ "session_id": "s1", "title": "first session" })),
    );
    assert!(resp.success, "create failed: {resp:?}");
    let row = resp.data.expect("data");
    assert_eq!(row["id"], "s1");
    assert_eq!(row["title"], "first session");
    assert_eq!(row["message_count"], 0);

    // Append + load
    let resp = round_trip(
        &mut client,
        &Request::new(Command::SessionAppend).with_params(json!({
            "session_id": "s1",
            "role": "user",
            "text": "hello",
        })),
    );
    assert!(resp.success, "append failed: {resp:?}");
    assert_eq!(resp.data.expect("data")["seq"], 1);

    let resp = round_trip(
        &mut client,
        &Request::new(Command::SessionLoadMessages).with_params(json!({
            "session_id": "s1",
            "limit": MAX_LIMIT,
        })),
    );
    assert!(resp.success, "load failed: {resp:?}");
    let msgs = resp.data.expect("data");
    assert_eq!(msgs.as_array().map(|a: &Vec<Value>| a.len()), Some(1));
    assert_eq!(msgs[0]["text"], "hello");
    assert_eq!(msgs[0]["role"], "user");

    // Get summary reflects the appended message.
    let resp = round_trip(
        &mut client,
        &Request::new(Command::SessionGet).with_params(json!({ "session_id": "s1" })),
    );
    assert!(resp.success);
    assert_eq!(resp.data.expect("data")["message_count"], 1);

    // List by query
    let resp = round_trip(
        &mut client,
        &Request::new(Command::SessionList).with_params(json!({
            "query": "first",
            "limit": 10,
        })),
    );
    assert!(resp.success);
    let rows = resp.data.expect("data");
    assert_eq!(rows.as_array().map(|a: &Vec<Value>| a.len()), Some(1));
    assert_eq!(rows[0]["id"], "s1");
}

#[test]
fn session_delete_removes_rows_from_the_on_disk_db() {
    let dir = TempDir::new().unwrap();
    let cfg = daemon_config_in(&dir, "delete");
    let daemon = Daemon::start(cfg).unwrap();
    let mut client = connect(&daemon.socket_addr().path().to_path_buf());

    let resp = round_trip(
        &mut client,
        &Request::new(Command::SessionCreate).with_params(json!({
            "session_id": "del",
            "title": "doomed",
        })),
    );
    assert!(resp.success);

    let resp = round_trip(
        &mut client,
        &Request::new(Command::SessionAppend).with_params(json!({
            "session_id": "del",
            "role": SessionRole::User.as_str(),
            "text": "x",
        })),
    );
    assert!(resp.success);

    let resp = round_trip(
        &mut client,
        &Request::new(Command::SessionDelete).with_params(json!({ "session_id": "del" })),
    );
    assert!(resp.success);
    assert_eq!(resp.data, Some(json!({ "deleted": true })));

    // Repeat delete is a no-op.
    let resp = round_trip(
        &mut client,
        &Request::new(Command::SessionDelete).with_params(json!({ "session_id": "del" })),
    );
    assert!(resp.success);
    assert_eq!(resp.data, Some(json!({ "deleted": false })));

    // Get now reports null (session gone) and load_messages returns [].
    let resp = round_trip(
        &mut client,
        &Request::new(Command::SessionGet).with_params(json!({ "session_id": "del" })),
    );
    assert!(resp.success);
    assert!(resp.data.is_none() || resp.data == Some(Value::Null));

    let resp = round_trip(
        &mut client,
        &Request::new(Command::SessionLoadMessages).with_params(json!({
            "session_id": "del",
            "limit": MAX_LIMIT,
        })),
    );
    assert!(resp.success);
    assert_eq!(resp.data, Some(json!([])));

    // And the daemon-internal handle sees the same state — proves the
    // delete reached the on-disk libSQL DB.
    assert!(daemon.sessions().session("del").unwrap().is_none());
}

#[test]
fn second_start_in_same_dir_reports_already_running() {
    let dir = TempDir::new().unwrap();
    let cfg = daemon_config_in(&dir, "single");
    let _daemon = Daemon::start(cfg.clone()).unwrap();

    match Daemon::start(cfg) {
        Ok(_) => panic!("second start must fail"),
        Err(daemon::DaemonError::AlreadyRunning { pid }) => {
            assert_eq!(pid, std::process::id());
        }
        Err(other) => panic!("expected AlreadyRunning, got {other:?}"),
    }
}

#[test]
fn drop_removes_socket_lock_and_pid_artifacts() {
    let dir = TempDir::new().unwrap();
    let cfg = daemon_config_in(&dir, "drop");
    let sock = cfg.socket_path.as_ref().unwrap().clone();
    let lock_path = {
        let mut s = sock.as_os_str().to_owned();
        s.push(".lock");
        PathBuf::from(s)
    };
    let pid_path = {
        let mut s = sock.as_os_str().to_owned();
        s.push(".pid");
        PathBuf::from(s)
    };

    {
        let daemon = Daemon::start(cfg).unwrap();
        assert!(sock.exists(), "socket file should exist while daemon runs");
        assert!(
            lock_path.exists(),
            "lock file should exist while daemon runs"
        );
        assert!(pid_path.exists(), "pid file should exist while daemon runs");
        // Trigger a graceful shutdown and confirm artifacts are cleaned.
        drop(daemon);
    }

    assert!(!sock.exists(), "socket file should be removed on drop");
    assert!(!lock_path.exists(), "lock file should be removed on drop");
    assert!(!pid_path.exists(), "pid file should be removed on drop");
}

#[test]
fn drop_then_reacquire_in_same_dir_succeeds() {
    let dir = TempDir::new().unwrap();
    let cfg = daemon_config_in(&dir, "reacquire");

    {
        let _d1 = Daemon::start(cfg.clone()).unwrap();
    }
    // After drop, lock + socket + pid are gone; second start works.
    let _d2 = Daemon::start(cfg).unwrap();
}

#[test]
fn client_disconnect_reconnect_keeps_state() {
    let dir = TempDir::new().unwrap();
    let cfg = daemon_config_in(&dir, "reconnect");
    let daemon = Daemon::start(cfg).unwrap();
    let sock = daemon.socket_addr().path().to_path_buf();

    // First connection: write one row.
    {
        let mut c = connect(&sock);
        let resp = round_trip(
            &mut c,
            &Request::new(Command::SessionCreate).with_params(json!({
                "session_id": "persist",
                "title": "t",
            })),
        );
        assert!(resp.success);
    }

    // Reconnect; the daemon's accept loop keeps serving.
    {
        let mut c = connect(&sock);
        let resp = round_trip(
            &mut c,
            &Request::new(Command::SessionGet).with_params(json!({
                "session_id": "persist",
            })),
        );
        assert!(resp.success);
        assert_eq!(resp.data.expect("data")["title"], "t");
    }
}

#[test]
fn unknown_command_is_a_protocol_error() {
    let dir = TempDir::new().unwrap();
    let cfg = daemon_config_in(&dir, "bad-cmd");
    let daemon = Daemon::start(cfg).unwrap();
    let mut client = connect(&daemon.socket_addr().path().to_path_buf());

    // Hand-rolled JSON with a type field the enum doesn't know.
    let payload = r#"{"id":"x","type":"nope"}"#;
    client.write_all(payload.as_bytes()).unwrap();
    client.write_all(b"\n").unwrap();

    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = client.read(&mut byte).expect("read");
        if n == 0 {
            panic!("server closed early");
        }
        if byte[0] == b'\n' {
            break;
        }
        buf.push(byte[0]);
    }
    let resp: Response = serde_json::from_slice(&buf).expect("parse");
    assert!(!resp.success);
    let err = resp.error.expect("error envelope");
    assert_eq!(err.code, "protocol");
}

#[test]
fn missing_required_field_returns_protocol_error() {
    let dir = TempDir::new().unwrap();
    let cfg = daemon_config_in(&dir, "missing");
    let daemon = Daemon::start(cfg).unwrap();
    let mut client = connect(&daemon.socket_addr().path().to_path_buf());

    let resp = round_trip(
        &mut client,
        // No `title` → protocol error.
        &Request::new(Command::SessionCreate).with_params(json!({ "session_id": "x" })),
    );
    assert!(!resp.success);
    let err: ResponseError = resp.error.expect("error");
    assert_eq!(err.code, "protocol");
    assert!(err.message.contains("title"));
}

#[test]
fn daemon_info_reports_pid_and_uptime() {
    let dir = TempDir::new().unwrap();
    let cfg = daemon_config_in(&dir, "info");
    let daemon = Daemon::start(cfg).unwrap();
    let mut client = connect(&daemon.socket_addr().path().to_path_buf());

    let resp = round_trip(&mut client, &Request::new(Command::Info).with_id("i"));
    assert!(resp.success);
    let data = resp.data.expect("data");
    assert_eq!(data["pid"], serde_json::json!(std::process::id()));
    assert!(data["started_at_ms"].as_i64().unwrap() > 0);
    assert!(data["uptime_ms"].as_i64().unwrap() >= 0);
    // No worker spawned yet → 0.
    assert_eq!(data["worker_pid"], serde_json::json!(0));
}

#[test]
fn concurrent_clients_serialize_on_the_worker_mutex() {
    // We don't spawn a real worker here; instead we exercise the lock by
    // hammering `daemon.ping` from many threads at once.  All responses
    // must arrive and the daemon must still be healthy afterwards.
    let dir = TempDir::new().unwrap();
    let cfg = daemon_config_in(&dir, "concurrent");
    let daemon = Daemon::start(cfg).unwrap();
    let sock = daemon.socket_addr().path().to_path_buf();

    let n_threads: usize = 8;
    let per_thread: usize = 25;
    let mut handles = Vec::new();
    let errors = Arc::new(Mutex::new(Vec::<String>::new()));
    for t in 0..n_threads {
        let sock = sock.clone();
        let errors = Arc::clone(&errors);
        handles.push(std::thread::spawn(move || {
            for i in 0..per_thread {
                let mut c = connect(&sock);
                let resp = round_trip(
                    &mut c,
                    &Request::new(Command::Ping).with_id(format!("p-{t}-{i}")),
                );
                if !resp.success {
                    errors.lock().push(format!("t={t} i={i} resp: {resp:?}"));
                }
            }
        }));
    }
    for h in handles {
        h.join().expect("thread join");
    }
    let errs = errors.lock().clone();
    assert!(errs.is_empty(), "concurrent ping errors: {errs:?}");

    // Daemon is still healthy after the storm.
    let mut c = connect(&sock);
    let resp = round_trip(&mut c, &Request::new(Command::Ping));
    assert!(resp.success);
}
#[test]
fn idle_client_does_not_block_other_clients() {
    let dir = TempDir::new().unwrap();
    let cfg = daemon_config_in(&dir, "idle-client");
    let daemon = Daemon::start(cfg).unwrap();
    let sock = daemon.socket_addr().path().to_path_buf();

    let _idle = connect(&sock);
    let mut active = connect(&sock);
    let resp = round_trip(&mut active, &Request::new(Command::Ping));
    assert!(resp.success);
}

#[test]
fn env_var_override_routes_session_db_and_socket() {
    let _guard = ENV_LOCK.lock();
    // ponytail: paths/override is exercised in config's tests; here we
    // just sanity-check `DaemonConfig::from_config` honors env overrides
    // by pointing OMENIC_DAEMON_SOCKET + OMENIC_SESSION_DB at a temp dir
    // and starting the daemon.
    let dir = TempDir::new().unwrap();
    let sock = dir.path().join("from-env.sock");
    let db = dir.path().join("from-env.db");
    unsafe {
        env::set_var("OMENIC_DAEMON_SOCKET", &sock);
        env::set_var("OMENIC_SESSION_DB", &db);
    }

    let cfg = config::Config::load().expect("config load");
    let daemon_cfg = DaemonConfig::from_config(&cfg).expect("from_config");
    let daemon = Daemon::start(daemon_cfg).expect("start");
    assert_eq!(daemon.socket_addr().path(), sock);
    assert!(sock.exists(), "env-overridden socket should be bound");

    unsafe {
        env::remove_var("OMENIC_DAEMON_SOCKET");
        env::remove_var("OMENIC_SESSION_DB");
    }
}
/// Test that the daemon's shutdown behavior is preserved when triggered via signals.
/// This test verifies that Daemon::shutdown() can be called to trigger graceful shutdown,
/// simulating what happens when a SIGINT/SIGTERM is received.
#[test]
fn shutdown_trigger_behaves_like_signal() {
    let dir = TempDir::new().unwrap();
    let cfg = daemon_config_in(&dir, "shutdown_signal");
    let sock = cfg.socket_path.as_ref().unwrap().clone();
    let lock_path = {
        let mut s = sock.as_os_str().to_owned();
        s.push(".lock");
        PathBuf::from(s)
    };
    let pid_path = {
        let mut s = sock.as_os_str().to_owned();
        s.push(".pid");
        PathBuf::from(s)
    };

    {
        let mut daemon = Daemon::start(cfg).unwrap();
        assert!(sock.exists(), "socket file should exist while daemon runs");
        assert!(
            lock_path.exists(),
            "lock file should exist while daemon runs"
        );
        assert!(pid_path.exists(), "pid file should exist while daemon runs");

        // Trigger shutdown like a signal handler would
        daemon.shutdown();
    }

    // After shutdown, artifacts should be cleaned up
    assert!(
        !sock.exists(),
        "socket file should be removed after shutdown"
    );
    assert!(
        !lock_path.exists(),
        "lock file should be removed after shutdown"
    );
    assert!(
        !pid_path.exists(),
        "pid file should be removed after shutdown"
    );
}

fn daemon_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_daemon"))
}

fn runtime_paths(socket: &std::path::Path) -> (PathBuf, PathBuf) {
    let mut lock = socket.as_os_str().to_owned();
    lock.push(".lock");
    let mut pid = socket.as_os_str().to_owned();
    pid.push(".pid");
    (PathBuf::from(lock), PathBuf::from(pid))
}

fn wait_for_path(path: &std::path::Path, present: bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while path.exists() != present && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(path.exists(), present, "path state: {}", path.display());
}

#[test]
fn daemon_binary_cleans_runtime_files_after_sigint_and_sigterm() {
    for signal in [libc::SIGINT, libc::SIGTERM] {
        let dir = TempDir::new().unwrap();
        let socket = dir.path().join(format!("signal-{signal}.sock"));
        let db = dir.path().join(format!("signal-{signal}.db"));
        let (lock, pid) = runtime_paths(&socket);
        let mut child = ProcessCommand::new(daemon_binary())
            .env("OMENIC_DAEMON_SOCKET", &socket)
            .env("OMENIC_SESSION_DB", &db)
            .env("OMENIC_OMP_PATH", "omp-not-installed-for-tests")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start daemon binary");

        wait_for_path(&socket, true);
        wait_for_path(&lock, true);
        wait_for_path(&pid, true);
        assert_eq!(unsafe { libc::kill(child.id() as i32, signal) }, 0);
        let status = child.wait().expect("wait for daemon");
        assert!(status.success(), "daemon exited: {status:?}");
        assert_eq!(status.signal(), None);
        wait_for_path(&socket, false);
        wait_for_path(&lock, false);
        wait_for_path(&pid, false);
    }
}
