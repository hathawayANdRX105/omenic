//! `session.update_title` e2e — the rename RPC added for the F2 regression:
//! a session created from the sidebar button persisted only the
//! `会话 <ts>` placeholder, so its first-message title lived in memory and
//! reverted on refresh.
//!
//! All three tests run against a real daemon on a temp socket with a real
//! sqlite file, exactly like `g6_e2e`. The mock OpenAI backend the shared
//! `common` fixture wires up is never called — no prompt is ever sent.

use daemon::protocol::{Command, Response};
use daemon::{Daemon, DaemonClient};
use serde_json::{Value, json};
use tempfile::tempdir;

use common::{MockOpenAi, daemon_cfg, one_text_turn};

mod common;

/// Start a daemon on `dir` and hand back a connected client. The mock LLM
/// backend exists only because `daemon_cfg` requires one; these tests never
/// prompt, so no request ever reaches it.
fn start_daemon(dir: &std::path::Path) -> (Daemon, DaemonClient) {
    let mock = MockOpenAi::start(vec![one_text_turn("unused")]);
    let cfg = daemon_cfg(dir, &mock, 8);
    let daemon = Daemon::start(cfg).expect("daemon start");
    let client = DaemonClient::connect_to(daemon.socket_addr().path());
    (daemon, client)
}

/// One JSONL round trip; panics with the transport error if the frame never
/// arrives (a hung daemon is a test bug, not a failure to assert on).
fn call(client: &DaemonClient, command: Command, params: Value) -> Response {
    client
        .call_raw(command, params)
        .expect("rpc round trip over the daemon socket")
}

fn str_field<'a>(data: &'a Value, field: &str) -> Option<&'a str> {
    data.get(field).and_then(Value::as_str)
}

/// The title written on the first message must survive a restart.
///
/// Red when: the handler returns a fabricated summary without executing the
/// UPDATE (or writes to some in-memory map / the wrong store) — the reopened
/// daemon then reads the `会话 <ts>` placeholder back, which is exactly the
/// F2 bug the RPC exists to fix.
#[test]
fn update_title_persists_across_reopen() {
    let dir = tempdir().expect("temp dir");
    let sid = "s-title-refresh";

    // Sidebar-button path: the row lands with the placeholder title.
    let (mut daemon, client) = start_daemon(dir.path());
    let created = call(
        &client,
        Command::SessionCreate,
        json!({ "session_id": sid, "title": "会话 1" }),
    );
    assert!(created.success, "create failed: {:?}", created.error);

    let updated = call(
        &client,
        Command::SessionUpdateTitle,
        json!({ "session_id": sid, "title": "首条消息截词标题" }),
    );
    assert!(updated.success, "update failed: {:?}", updated.error);
    assert_eq!(
        updated.data.as_ref().and_then(|d| str_field(d, "title")),
        Some("首条消息截词标题"),
        "the response summary must carry the new title"
    );

    drop(client);
    daemon.shutdown();
    // Drop releases the instance lock + socket files; the second start needs
    // both free (same dir, same socket path).
    drop(daemon);

    // Fresh daemon over the same DB file: only a real UPDATE survived.
    let (mut daemon2, client2) = start_daemon(dir.path());
    let got = call(&client2, Command::SessionGet, json!({ "session_id": sid }));
    assert!(got.success, "get failed: {:?}", got.error);
    assert_eq!(
        got.data.as_ref().and_then(|d| str_field(d, "title")),
        Some("首条消息截词标题"),
        "reopened daemon must read the persisted title, not the placeholder"
    );
    drop(client2);
    daemon2.shutdown();
}

/// Renaming a session that does not exist is an error, and no row appears.
///
/// Red when: `update_title` is (or degrades into) an upsert — the call
/// succeeds and a ghost row shows up in `session.get` / `session.list`,
/// masking the caller's bug of updating a session it never created.
#[test]
fn update_title_missing_session_errors() {
    let dir = tempdir().expect("temp dir");
    let (mut daemon, client) = start_daemon(dir.path());

    let created = call(
        &client,
        Command::SessionCreate,
        json!({ "session_id": "real", "title": "real session" }),
    );
    assert!(created.success, "create failed: {:?}", created.error);

    let ghost = call(
        &client,
        Command::SessionUpdateTitle,
        json!({ "session_id": "ghost", "title": "should not exist" }),
    );
    assert!(
        !ghost.success,
        "renaming a missing session must fail, got {:?}",
        ghost.data
    );
    assert_eq!(
        ghost.error.as_ref().map(|e| e.code.as_str()),
        Some("database_missing"),
        "the store's missing-row error must reach the client"
    );

    // Update is not an upsert: nothing was inserted for `ghost`.
    let got = call(
        &client,
        Command::SessionGet,
        json!({ "session_id": "ghost" }),
    );
    assert!(got.success, "get failed: {:?}", got.error);
    // A miss deserializes as `data: None` (serde maps explicit null on an
    // Option to None); an upserted ghost row would come back as a summary.
    assert!(
        got.data.is_none(),
        "missing session must stay missing, got {:?}",
        got.data
    );
    let list = call(
        &client,
        Command::SessionList,
        json!({ "query": "%", "limit": 50 }),
    );
    assert!(list.success, "list failed: {:?}", list.error);
    let titles: Vec<&str> = list
        .data
        .as_ref()
        .and_then(Value::as_array)
        .expect("list returns an array")
        .iter()
        .filter_map(|row| str_field(row, "title"))
        .collect();
    assert_eq!(
        titles,
        vec!["real session"],
        "no ghost row may appear in the session list"
    );

    drop(client);
    daemon.shutdown();
}

/// The rename touches `title` (and `updated_at`) and nothing else.
///
/// Red when: the UPDATE statement also rewrites other columns (or the row is
/// deleted and re-inserted) — `parent_id`, `created_at_ms`, and the appended
/// message would not survive the rename.
#[test]
fn update_title_does_not_touch_other_columns() {
    let dir = tempdir().expect("temp dir");
    let (mut daemon, client) = start_daemon(dir.path());

    let created = call(
        &client,
        Command::SessionCreate,
        json!({ "session_id": "s-cols", "title": "占位标题", "parent_id": "p-1" }),
    );
    assert!(created.success, "create failed: {:?}", created.error);
    let before = created.data.expect("create returns the summary");
    let created_at = before
        .get("created_at_ms")
        .and_then(Value::as_i64)
        .expect("created_at_ms");
    assert_eq!(
        before.get("parent_id").and_then(Value::as_str),
        Some("p-1"),
        "fixture: the session must start with a lineage edge"
    );

    // One message so `message_count` is a checked column too.
    let appended = call(
        &client,
        Command::SessionAppend,
        json!({ "session_id": "s-cols", "role": "user", "text": "hello" }),
    );
    assert!(appended.success, "append failed: {:?}", appended.error);

    let updated = call(
        &client,
        Command::SessionUpdateTitle,
        json!({ "session_id": "s-cols", "title": "新标题" }),
    );
    assert!(updated.success, "update failed: {:?}", updated.error);
    let after = updated.data.expect("update returns the summary");

    assert_eq!(
        str_field(&after, "title"),
        Some("新标题"),
        "the rename must actually land"
    );
    assert_eq!(
        after.get("parent_id").and_then(Value::as_str),
        Some("p-1"),
        "the lineage edge must survive the rename"
    );
    assert_eq!(
        after.get("created_at_ms").and_then(Value::as_i64),
        Some(created_at),
        "created_at must not move"
    );
    assert_eq!(
        after.get("message_count").and_then(Value::as_u64),
        Some(1),
        "the appended message must survive the rename"
    );

    drop(client);
    daemon.shutdown();
}
