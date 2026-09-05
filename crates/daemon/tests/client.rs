//! Integration tests for the daemon client API.
//!
//! Exercises the client against a real Unix-domain socket (spawned in a
//! tempdir). Covers command JSON shape, connection failure, deletion,
//! reconnect, and the registered `session_query` ToolDef.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use daemon::protocol::{Command, Request, Response};
use daemon::session_query::{SESSION_QUERY_NAME, session_query_def};
use daemon::{AppendOutcome, ClientError, Daemon, DaemonClient, DaemonConfig};
use serde_json::{Value, json};
use session::{SessionMessage, SessionRole, SessionSummary};
use tempfile::TempDir;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn daemon_config_in(dir: &TempDir, tag: &str) -> DaemonConfig {
    DaemonConfig {
        socket_path: Some(dir.path().join(format!("{tag}.sock"))),
        omp_path: "omp-not-installed-for-tests".to_string(),
        session_db_path: Some(dir.path().join(format!("{tag}.db"))),
    }
}

fn connect(sock: &PathBuf) -> UnixStream {
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

#[test]
fn connect_to_nonexistent_socket_returns_connect_error() {
    let client = DaemonClient::connect_to("/nonexistent/path/missing.sock");
    match client.ping() {
        Err(ClientError::Connect(_)) => {}
        Err(other) => panic!("unexpected error: {other:?}"),
        Ok(_) => panic!("connect to nonexistent socket must fail"),
    }
}

#[test]
fn ping_round_trip_through_client() {
    let dir = TempDir::new().unwrap();
    let cfg = daemon_config_in(&dir, "ping");
    let daemon = daemon::Daemon::start(cfg).unwrap();
    let client = DaemonClient::connect_to(daemon.socket_addr().path().to_path_buf());
    assert!(client.ping().unwrap(), "ping should return true");
}

#[test]
fn info_reports_pid_and_uptime() {
    let dir = TempDir::new().unwrap();
    let cfg = daemon_config_in(&dir, "info");
    let daemon = daemon::Daemon::start(cfg).unwrap();
    let client = DaemonClient::connect_to(daemon.socket_addr().path().to_path_buf());
    let info = client.info().unwrap();
    assert_eq!(info.pid, std::process::id());
    assert!(info.started_at_ms > 0);
    assert!(info.uptime_ms >= 0);
}

#[test]
fn run_list_returns_the_latest_persisted_records() {
    let dir = TempDir::new().unwrap();
    let cfg = daemon_config_in(&dir, "runs");
    let daemon = daemon::Daemon::start(cfg).unwrap();
    daemon.runs().start("run-1", "session-1", 1).unwrap();
    daemon.runs().start("run-2", "session-1", 2).unwrap();
    daemon.runs().start("run-3", "session-1", 3).unwrap();

    let client = DaemonClient::connect_to(daemon.socket_addr().path().to_path_buf());
    let runs = client.run_list(2).unwrap();
    assert_eq!(
        runs.iter()
            .map(|run| run.run_id.as_str())
            .collect::<Vec<_>>(),
        ["run-2", "run-3"]
    );
}

#[test]
fn session_crud_through_client_returns_typed_summaries() {
    let dir = TempDir::new().unwrap();
    let cfg = daemon_config_in(&dir, "crud");
    let daemon = daemon::Daemon::start(cfg).unwrap();
    let client = DaemonClient::connect_to(daemon.socket_addr().path().to_path_buf());

    // Create.
    let created: SessionSummary = client.session_create("s1", "first").unwrap();
    assert_eq!(created.id, "s1");
    assert_eq!(created.title, "first");
    assert_eq!(created.message_count, 0);

    // Get — message_count is still 0.
    let row: Option<SessionSummary> = client.session_get("s1").unwrap();
    let row = row.expect("session present");
    assert_eq!(row.id, "s1");
    assert_eq!(row.message_count, 0);

    // Append a message.
    let appended: AppendOutcome = client
        .session_append("s1", SessionRole::User, "hello")
        .unwrap();
    assert_eq!(appended.seq, 1);

    // List by query.
    let rows: Vec<SessionSummary> = client.session_list("first", 10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "s1");
    assert_eq!(rows[0].message_count, 1);

    // Load messages.
    let msgs: Vec<SessionMessage> = client.session_load_messages("s1", 100).unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].role, SessionRole::User);
    assert_eq!(msgs[0].text, "hello");
    assert_eq!(msgs[0].seq, 1);

    // Search across messages.
    let hits: Vec<SessionMessage> = client.session_search("hello", None, 100).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].text, "hello");

    let scoped: Vec<SessionMessage> = client.session_search("hello", Some("s1"), 100).unwrap();
    assert_eq!(scoped.len(), 1);
}

#[test]
fn session_delete_through_client_returns_deleted_flag() {
    let dir = TempDir::new().unwrap();
    let cfg = daemon_config_in(&dir, "delete");
    let daemon = daemon::Daemon::start(cfg).unwrap();
    let client = DaemonClient::connect_to(daemon.socket_addr().path().to_path_buf());

    // No row → false.
    assert!(!client.session_delete("nope").unwrap());

    client.session_create("kill", "t").unwrap();
    client
        .session_append("kill", SessionRole::User, "bye")
        .unwrap();

    // Now exists → true.
    assert!(client.session_delete("kill").unwrap());
    // Repeat → false.
    assert!(!client.session_delete("kill").unwrap());

    // Get returns None after delete.
    let row: Option<SessionSummary> = client.session_get("kill").unwrap();
    assert!(row.is_none());
}

#[test]
fn reconnect_after_drop_preserves_state() {
    let dir = TempDir::new().unwrap();
    let cfg = daemon_config_in(&dir, "reconnect");
    let daemon = daemon::Daemon::start(cfg).unwrap();
    let sock = daemon.socket_addr().path().to_path_buf();

    // First connection: create + append.
    {
        let client = DaemonClient::connect_to(sock.clone());
        client.session_create("keep", "t").unwrap();
        client
            .session_append("keep", SessionRole::User, "first")
            .unwrap();
    }

    // Second connection: re-read state.
    let client2 = DaemonClient::connect_to(sock.clone());
    let row: Option<SessionSummary> = client2.session_get("keep").unwrap();
    let row = row.expect("row should be present after reconnect");
    assert_eq!(row.id, "keep");
    assert_eq!(row.message_count, 1);

    let msgs = client2.session_load_messages("keep", 100).unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].text, "first");
}

#[test]
fn session_search_scope_is_preserved_on_the_wire() {
    let dir = TempDir::new().unwrap();
    let cfg = daemon_config_in(&dir, "search-scope");
    let daemon = daemon::Daemon::start(cfg).unwrap();
    let client = DaemonClient::connect_to(daemon.socket_addr().path().to_path_buf());
    client.session_create("a", "a").unwrap();
    client.session_create("b", "b").unwrap();
    client
        .session_append("a", SessionRole::User, "shared text")
        .unwrap();
    client
        .session_append("b", SessionRole::User, "shared text")
        .unwrap();

    let rows = client.session_search("shared", Some("a"), 10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].session_id, "a");
}

#[test]
fn unknown_command_is_a_structured_error() {
    let dir = TempDir::new().unwrap();
    let cfg = daemon_config_in(&dir, "bad");
    let daemon = daemon::Daemon::start(cfg).unwrap();
    let sock = daemon.socket_addr().path().to_path_buf();
    let mut stream = connect(&sock);

    let resp = round_trip(
        &mut stream,
        &Request {
            id: Some("x".into()),
            command: Command::SessionGet,
            params: json!({}), // missing session_id
        },
    );
    assert!(!resp.success);
    let err = resp.error.expect("error envelope");
    assert_eq!(err.code, "protocol");
}

#[test]
fn session_query_dispatches_each_kind() {
    let dir = TempDir::new().unwrap();
    let cfg = daemon_config_in(&dir, "query");
    let daemon = daemon::Daemon::start(cfg).unwrap();
    let client = DaemonClient::connect_to(daemon.socket_addr().path().to_path_buf());

    client.session_create("a1", "alpha").unwrap();
    client.session_create("a2", "beta").unwrap();
    client
        .session_append("a1", SessionRole::Assistant, "first message")
        .unwrap();

    // kind=list.
    let rows: Value = client
        .session_query(&json!({ "kind": "list", "query": "alpha", "limit": 10 }))
        .unwrap();
    let arr = rows.as_array().expect("list returns array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "a1");

    // kind=get.
    let got: Value = client
        .session_query(&json!({ "kind": "get", "session_id": "a1" }))
        .unwrap();
    assert_eq!(got["id"], "a1");

    // kind=search.
    let hits: Value = client
        .session_query(&json!({ "kind": "search", "query": "first", "limit": 10 }))
        .unwrap();
    let hits = hits.as_array().expect("search returns array");
    assert_eq!(hits.len(), 1);

    // kind=delete.
    let deleted: Value = client
        .session_query(&json!({ "kind": "delete", "session_id": "a2" }))
        .unwrap();
    assert_eq!(deleted["deleted"], json!(true));

    // After delete, kind=get returns null.
    let gone: Value = client
        .session_query(&json!({ "kind": "get", "session_id": "a2" }))
        .unwrap();
    assert!(gone.is_null());
}

#[test]
fn session_query_rejects_bad_input_before_connecting() {
    let client = DaemonClient::connect_to("/nonexistent/missing.sock");
    // Missing `kind` is a protocol error and never touches the socket.
    let err = client.session_query(&json!({ "query": "x" })).unwrap_err();
    assert!(matches!(err, ClientError::Protocol(_)), "{err:?}");

    let err = client
        .session_query(&json!({ "kind": "nuke" }))
        .unwrap_err();
    assert!(matches!(err, ClientError::Protocol(_)), "{err:?}");
}

#[test]
fn session_query_def_is_stable() {
    // The agent sees exactly one ToolDef. The def is the daemon's
    // registration payload sent to omp; pinning its name and required
    // fields here is what keeps the agent's schema honest.
    let def = session_query_def();
    assert_eq!(def.name, SESSION_QUERY_NAME);
    assert_eq!(def.name, "session_query");
    assert_eq!(def.parameters["required"][0], "kind");
    let kinds = def.parameters["properties"]["kind"]["enum"]
        .as_array()
        .expect("kind enum");
    for k in ["list", "get", "search", "delete"] {
        assert!(kinds.iter().any(|v| v == k), "missing kind {k}");
    }
}

#[test]
fn protocol_round_trip_preserves_session_payload_shape() {
    // Hand-rolled JSON round-trip on the wire: a SessionSummary with
    // all four fields the daemon emits must survive serialize → parse.
    let dir = TempDir::new().unwrap();
    let cfg = daemon_config_in(&dir, "shape");
    let daemon = daemon::Daemon::start(cfg).unwrap();
    let sock = daemon.socket_addr().path().to_path_buf();
    let mut stream = connect(&sock);

    let create = round_trip(
        &mut stream,
        &Request::new(Command::SessionCreate)
            .with_id("c1")
            .with_params(json!({ "session_id": "shape", "title": "t" })),
    );
    assert!(create.success);
    let raw = serde_json::to_value(&create.data.expect("data")).unwrap();
    assert_eq!(raw["id"], "shape");
    assert_eq!(raw["title"], "t");
    assert_eq!(raw["message_count"], 0);
    assert!(raw["created_at_ms"].as_i64().unwrap() > 0);
    assert!(raw["updated_at_ms"].as_i64().unwrap() > 0);

    let get = round_trip(
        &mut stream,
        &Request::new(Command::SessionGet).with_params(json!({ "session_id": "shape" })),
    );
    assert!(get.success);
    let get_raw = serde_json::to_value(&get.data.expect("data")).unwrap();
    assert_eq!(get_raw["id"], "shape");
}

#[test]
fn env_var_override_routes_client_socket() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // The client honors OMENIC_DAEMON_SOCKET via Config::daemon_socket_path.
    let dir = TempDir::new().unwrap();
    let sock = dir.path().join("client-env.sock");
    let db = dir.path().join("client-env.db");
    unsafe {
        std::env::set_var("OMENIC_DAEMON_SOCKET", &sock);
        std::env::set_var("OMENIC_SESSION_DB", &db);
    }
    let cfg = config::Config::load().expect("config load");
    let daemon_cfg = DaemonConfig::from_config(&cfg).expect("from_config");
    let daemon = daemon::Daemon::start(daemon_cfg).expect("start");
    let client = DaemonClient::from_config(&cfg).expect("client");
    assert_eq!(client.socket_path(), sock);
    client.session_create("env", "t").unwrap();

    drop(daemon);
    unsafe {
        std::env::remove_var("OMENIC_DAEMON_SOCKET");
        std::env::remove_var("OMENIC_SESSION_DB");
    }
}
