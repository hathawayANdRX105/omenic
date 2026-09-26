//! `session.truncate` e2e — the rewrite primitive behind T12 retry / edit
//! (issue #473): drop every message with `seq >= from_seq`, report how many
//! rows actually went away, then let the caller append the replacement.
//!
//! The count is the truth: a no-op range answers `0`, never a fabricated
//! success. All tests run against a real daemon on a temp socket with a real
//! sqlite file — the mock OpenAI backend the shared `common` fixture wires up
//! is never called, because no prompt is ever sent.

use daemon::protocol::{Command, Response};
use daemon::{Daemon, DaemonClient};
use serde_json::{Value, json};
use session::SessionRole;
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
        json!({ "session_id": session_id, "title": "truncate fixture" }),
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

/// Truncating the tail drops `seq >= from_seq` and keeps the head.
///
/// Red when: the handler deletes the whole session, deletes `seq < from_seq`,
/// or answers a count that disagrees with what `session.load_messages` reads
/// back — retry/edit would then replay from the wrong point.
#[test]
fn truncate_drops_tail_from_from_seq() {
    let dir = tempdir().expect("temp dir");
    let (mut daemon, client) = start_daemon(dir.path());
    let seqs = seed(&client, "s-trunc", &["one", "two", "three"]);
    assert_eq!(seqs, vec![1, 2, 3], "seqs must ascend from 1");

    let deleted = client.session_truncate("s-trunc", 2).expect("truncate");
    assert_eq!(deleted, 2, "the two tail rows go away");

    let kept = client
        .session_load_messages("s-trunc", 10)
        .expect("load_messages");
    assert_eq!(kept.len(), 1, "only the head survives");
    assert_eq!(kept[0].seq, 1);
    assert_eq!(kept[0].text, "one");

    drop(client);
    daemon.shutdown();
}

/// `from_seq <= 0` empties the session (documented boundary).
#[test]
fn truncate_from_zero_empties_session() {
    let dir = tempdir().expect("temp dir");
    let (mut daemon, client) = start_daemon(dir.path());
    seed(&client, "s-zero", &["one", "two", "three"]);

    let deleted = client.session_truncate("s-zero", 0).expect("truncate");
    assert_eq!(deleted, 3, "every row is in range");

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
}

/// A `from_seq` past the tail deletes nothing and says so (`0`), with the
/// messages intact — truncating ahead of the end must not eat history.
#[test]
fn truncate_past_tail_is_a_noop() {
    let dir = tempdir().expect("temp dir");
    let (mut daemon, client) = start_daemon(dir.path());
    seed(&client, "s-past", &["one", "two"]);

    let deleted = client.session_truncate("s-past", 99).expect("truncate");
    assert_eq!(deleted, 0, "nothing in range");

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
}

/// Truncating twice is the same as truncating once (idempotent tail cut —
/// retry failing halfway must not corrupt a second attempt).
#[test]
fn truncate_is_idempotent() {
    let dir = tempdir().expect("temp dir");
    let (mut daemon, client) = start_daemon(dir.path());
    seed(&client, "s-idem", &["one", "two", "three"]);

    let first = client.session_truncate("s-idem", 2).expect("truncate");
    assert_eq!(first, 2);
    let second = client.session_truncate("s-idem", 2).expect("truncate");
    assert_eq!(second, 0, "second cut finds nothing left in range");

    let kept = client
        .session_load_messages("s-idem", 10)
        .expect("load_messages");
    assert_eq!(kept.len(), 1, "head still intact");

    drop(client);
    daemon.shutdown();
}

/// Truncating an unknown session reports `0` rows instead of erroring or
/// inventing a session — the count is the truth (protocol doc contract).
#[test]
fn truncate_unknown_session_reports_zero() {
    let dir = tempdir().expect("temp dir");
    let (mut daemon, client) = start_daemon(dir.path());

    let deleted = client.session_truncate("ghost", 1).expect("truncate");
    assert_eq!(deleted, 0, "no rows, no fabricated success");

    drop(client);
    daemon.shutdown();
}

/// `from_seq` is required and never defaulted: a missing field is a
/// `protocol` error, not a silent `from_seq = 0` that would empty the
/// session on a client bug.
#[test]
fn truncate_requires_from_seq() {
    let dir = tempdir().expect("temp dir");
    let (mut daemon, client) = start_daemon(dir.path());
    seed(&client, "s-required", &["one", "two"]);

    let bad = call(
        &client,
        Command::SessionTruncate,
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
}
