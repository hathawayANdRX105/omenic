//! `session.rewind` e2e — the T13 turn-rewind primitive (issue #474): inside
//! one transaction, copy every message with `seq >= from_seq` into the rewind
//! backup table (stamped `SNAPSHOT_IDENTITY`), then delete the same range.
//!
//! The contract under test: both halves always land together or not at all,
//! the boundary semantics are `session.truncate`'s verbatim, the backup rows
//! carry the identity label, and `session.truncate` itself keeps its old
//! behavior. All tests run against a real daemon on a temp socket with a real
//! sqlite file — the mock OpenAI backend the shared `common` fixture wires up
//! is never called, because no prompt is ever sent.

use daemon::protocol::{Command, Response};
use daemon::{Daemon, DaemonClient};
use serde_json::{Value, json};
use session::{SNAPSHOT_IDENTITY, SessionDb, SessionRole};
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

/// Create `session_id` and seed it with `texts`, returning their assigned
/// seqs (ascending from 1).
fn seed(client: &DaemonClient, session_id: &str, texts: &[&str]) -> Vec<i64> {
    let created = call(
        client,
        Command::SessionCreate,
        json!({ "session_id": session_id, "title": "rewind fixture" }),
    );
    assert!(created.success, "create failed: {:?}", created.error);
    texts
        .iter()
        .map(|text| {
            client
                .session_append(session_id, SessionRole::User, text, &[])
                .expect("append")
                .seq
        })
        .collect()
}

/// Stop the daemon, reopen the sqlite file directly, and read back the audit
/// seam: rows in the rewind backup **carrying `SNAPSHOT_IDENTITY`** for
/// `session_id`. Reading post-shutdown keeps the assertions off the daemon's
/// live connection.
fn snapshot_count(dir: &std::path::Path, session_id: &str) -> u64 {
    let db = SessionDb::open(dir.join("sessions.db")).expect("reopen db");
    db.rewind_snapshot_count(session_id)
        .expect("read rewind snapshot count")
}

/// Both halves land together: the tail is dropped **and** the same rows show
/// up in the backup with the identity label.
///
/// Red when: the handler deletes without snapshotting (backup count stays
/// `0`), snapshots without deleting (history survives), or answers a count
/// that disagrees with what `session.load_messages` reads back.
#[test]
fn rewind_drops_tail_and_backs_it_up() {
    let dir = tempdir().expect("temp dir");
    let (mut daemon, client) = start_daemon(dir.path());
    let seqs = seed(&client, "s-rewind", &["one", "two", "three"]);
    assert_eq!(seqs, vec![1, 2, 3], "seqs must ascend from 1");

    let snapshotted = client.session_rewind("s-rewind", 2).expect("rewind");
    assert_eq!(snapshotted, 2, "the two tail rows are backed up");

    let kept = client
        .session_load_messages("s-rewind", 10)
        .expect("load_messages");
    assert_eq!(kept.len(), 1, "only the head survives");
    assert_eq!(kept[0].seq, 1);
    assert_eq!(kept[0].text, "one");

    drop(client);
    daemon.shutdown();
    assert_eq!(
        snapshot_count(dir.path(), "s-rewind"),
        2,
        "the backup holds the two discarded rows, stamped {SNAPSHOT_IDENTITY:?}"
    );
}

/// `from_seq <= 0` empties the session and snapshots everything (documented
/// boundary, `session.truncate`'s semantics carried over verbatim).
#[test]
fn rewind_from_zero_empties_and_snapshots_everything() {
    let dir = tempdir().expect("temp dir");
    let (mut daemon, client) = start_daemon(dir.path());
    seed(&client, "s-zero", &["one", "two", "three"]);

    let snapshotted = client.session_rewind("s-zero", 0).expect("rewind");
    assert_eq!(snapshotted, 3, "every row is in range");

    let kept = client
        .session_load_messages("s-zero", 10)
        .expect("load_messages");
    assert!(
        kept.is_empty(),
        "session must be empty, got {:?}",
        kept.len()
    );

    drop(client);
    daemon.shutdown();
    assert_eq!(
        snapshot_count(dir.path(), "s-zero"),
        3,
        "the emptied rows are all in the backup"
    );
}

/// A `from_seq` past the tail touches nothing and says so (`0`) — messages
/// intact, backup empty. Rewinding ahead of the end must not eat history or
/// fabricate a snapshot.
#[test]
fn rewind_past_tail_is_a_noop() {
    let dir = tempdir().expect("temp dir");
    let (mut daemon, client) = start_daemon(dir.path());
    seed(&client, "s-past", &["one", "two"]);

    let snapshotted = client.session_rewind("s-past", 99).expect("rewind");
    assert_eq!(snapshotted, 0, "nothing in range");

    let kept = client
        .session_load_messages("s-past", 10)
        .expect("load_messages");
    assert_eq!(
        kept.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
        vec!["one", "two"],
        "history untouched"
    );

    drop(client);
    daemon.shutdown();
    assert_eq!(
        snapshot_count(dir.path(), "s-past"),
        0,
        "a no-op range must not write backup rows"
    );
}

/// Rewinding an unknown session answers `0` rows instead of erroring or
/// inventing a session — the count is the truth (same contract as
/// `session.truncate`).
#[test]
fn rewind_unknown_session_reports_zero() {
    let dir = tempdir().expect("temp dir");
    let (mut daemon, client) = start_daemon(dir.path());

    let snapshotted = client.session_rewind("ghost", 1).expect("rewind");
    assert_eq!(snapshotted, 0, "no rows, no fabricated success");

    drop(client);
    daemon.shutdown();
    assert_eq!(snapshot_count(dir.path(), "ghost"), 0);
}

/// `from_seq` is required and never defaulted: a missing field is a
/// `protocol` error, not a silent `from_seq = 0` that would snapshot-and-
/// empty the session on a client bug.
#[test]
fn rewind_requires_from_seq() {
    let dir = tempdir().expect("temp dir");
    let (mut daemon, client) = start_daemon(dir.path());
    seed(&client, "s-required", &["one", "two"]);

    let bad = call(
        &client,
        Command::SessionRewind,
        json!({ "session_id": "s-required" }),
    );
    assert!(!bad.success, "missing from_seq must be rejected");
    assert_eq!(
        bad.error.as_ref().map(|e| e.code.as_str()),
        Some("protocol"),
        "the rejection must surface as a protocol error, got {:?}",
        bad.error
    );

    let kept = client
        .session_load_messages("s-required", 10)
        .expect("load_messages");
    assert_eq!(kept.len(), 2, "a rejected call must delete nothing");

    drop(client);
    daemon.shutdown();
    assert_eq!(
        snapshot_count(dir.path(), "s-required"),
        0,
        "a rejected call must not snapshot anything"
    );
}

/// Backup rows are append-only: rewinding the same range twice stacks a
/// second backup generation instead of colliding with the first, so each
/// discarded generation stays auditable.
#[test]
fn rewind_twice_stacks_snapshots() {
    let dir = tempdir().expect("temp dir");
    let (mut daemon, client) = start_daemon(dir.path());
    seed(&client, "s-stack", &["one", "two", "three"]);

    let first = client.session_rewind("s-stack", 3).expect("rewind #1");
    assert_eq!(first, 1, "first rewind backs up seq 3");
    let second = client.session_rewind("s-stack", 2).expect("rewind #2");
    assert_eq!(second, 1, "second rewind backs up seq 2");

    let kept = client
        .session_load_messages("s-stack", 10)
        .expect("load_messages");
    assert_eq!(kept.len(), 1, "head still intact");

    drop(client);
    daemon.shutdown();
    assert_eq!(
        snapshot_count(dir.path(), "s-stack"),
        2,
        "two rewind generations stack instead of colliding"
    );
}

/// `session.truncate` keeps its old contract after the T13 additions
/// (spec: truncate semantics untouched): it deletes the tail, reports the
/// count, and writes **no** backup rows — the backup belongs to rewind alone.
#[test]
fn truncate_keeps_its_contract_and_writes_no_backup() {
    let dir = tempdir().expect("temp dir");
    let (mut daemon, client) = start_daemon(dir.path());
    seed(&client, "s-regress", &["one", "two", "three"]);

    let deleted = client.session_truncate("s-regress", 2).expect("truncate");
    assert_eq!(deleted, 2, "truncate still reports the dropped tail");

    let kept = client
        .session_load_messages("s-regress", 10)
        .expect("load_messages");
    assert_eq!(kept.len(), 1, "truncate still keeps the head");

    drop(client);
    daemon.shutdown();
    assert_eq!(
        snapshot_count(dir.path(), "s-regress"),
        0,
        "truncate must not write rewind backup rows"
    );
}
