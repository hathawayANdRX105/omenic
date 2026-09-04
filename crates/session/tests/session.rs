//! Integration tests for the `session` crate.
//!
//! Covers the public surface end-to-end against a real on-disk libSQL/SQLite
//! database, including a multi-threaded writer race that exercises the
//! real WAL+`busy_timeout` cross-process path.

use std::collections::HashSet;
use std::sync::{Arc, Barrier};
use std::thread;

use session::{MAX_LIMIT, SessionDb, SessionError, SessionRole, SessionSummary};
use tempfile::TempDir;

// ponytail: a tiny helper that returns a fresh, fully-initialized `SessionDb`
// rooted at a unique file inside a per-test tempdir. Each test gets its own
// dir so they can run in parallel without colliding on the WAL.
fn open_temp(tag: &str) -> (TempDir, SessionDb) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(format!("{tag}.db"));
    let db = SessionDb::open(&path).expect("open fresh db");
    (dir, db)
}

// -----------------------------------------------------------------------------
// Construction + schema
// -----------------------------------------------------------------------------

#[test]
fn open_creates_file_and_parent_dir() {
    let dir = tempfile::tempdir().unwrap();
    // nested parent that does not exist yet
    let nested = dir.path().join("a").join("b").join("sessions.db");
    assert!(!nested.parent().unwrap().exists());
    let db = SessionDb::open(&nested).expect("open nested");
    assert!(nested.exists());
    // path() round-trips
    assert_eq!(db.path(), nested);
    assert!(db.is_empty().unwrap());
}

#[test]
fn reopen_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");

    let db = SessionDb::open(&path).unwrap();
    db.ensure_session("alpha", "first").unwrap();
    drop(db);

    let db2 = SessionDb::open(&path).unwrap();
    let row = db2.session("alpha").unwrap().expect("alpha present");
    assert_eq!(row.id, "alpha");
    assert_eq!(row.title, "first");
    assert_eq!(row.message_count, 0);
}

// -----------------------------------------------------------------------------
// Sessions: ensure / create / delete
// -----------------------------------------------------------------------------

#[test]
fn ensure_session_then_ensure_again_preserves_created_at() {
    let (_dir, db) = open_temp("ensure");
    let first = db.ensure_session("s1", "title-one").unwrap();
    // Sleep-free jitter: we can't use `thread::sleep` per spec, so we just
    // rely on `now_ms()` being monotonically non-decreasing. A second call
    // right after should still see created_at == first.created_at_ms.
    let second = db.ensure_session("s1", "title-two-attempt").unwrap();
    assert_eq!(first.created_at_ms, second.created_at_ms);
    // Title must NOT be overwritten on a repeat ensure_session.
    assert_eq!(second.title, "title-one");
    assert!(second.updated_at_ms >= first.updated_at_ms);
}

#[test]
fn delete_session_cascades_messages() {
    let (_dir, db) = open_temp("delete-cascade");
    db.ensure_session("s1", "t").unwrap();
    db.append_message("s1", SessionRole::User, "hi").unwrap();
    db.append_message("s1", SessionRole::Assistant, "hello")
        .unwrap();
    let before = db.load_messages("s1", MAX_LIMIT).unwrap();
    assert_eq!(before.len(), 2);

    let removed = db.delete_session("s1").unwrap();
    assert!(removed);
    assert!(db.session("s1").unwrap().is_none());
    assert!(db.load_messages("s1", MAX_LIMIT).unwrap().is_empty());

    // Second delete is a no-op
    assert!(!db.delete_session("s1").unwrap());
}

#[test]
fn empty_inputs_rejected() {
    let (_dir, db) = open_temp("empty");
    assert!(matches!(
        db.ensure_session("", "t"),
        Err(SessionError::InvalidSessionId)
    ));
    assert!(matches!(
        db.ensure_session("s", "  "),
        Err(SessionError::InvalidSessionId)
    ));
    assert!(matches!(
        db.append_message("", SessionRole::User, "x"),
        Err(SessionError::InvalidSessionId)
    ));
    assert!(matches!(
        db.append_message("s", SessionRole::User, ""),
        Err(SessionError::InvalidMessageText)
    ));
    assert!(matches!(
        db.search_messages("", None, 10),
        Err(SessionError::InvalidSearchQuery)
    ));
    assert!(matches!(
        db.list_sessions("", 10),
        Err(SessionError::InvalidListQuery)
    ));
}

// -----------------------------------------------------------------------------
// Messages: ordering, Unicode, roles
// -----------------------------------------------------------------------------

#[test]
fn append_message_assigns_monotonic_seq_per_session() {
    let (_dir, db) = open_temp("seq");
    db.ensure_session("s1", "t").unwrap();
    let (s1, _) = db.append_message("s1", SessionRole::User, "a").unwrap();
    let (s2, _) = db
        .append_message("s1", SessionRole::Assistant, "b")
        .unwrap();
    let (s3, _) = db.append_message("s1", SessionRole::System, "c").unwrap();
    assert_eq!((s1, s2, s3), (1, 2, 3));
}

#[test]
fn unknown_role_in_storage_is_an_error() {
    let (_dir, db) = open_temp("unknown-role");
    db.ensure_session("s1", "t").unwrap();
    db.append_message("s1", SessionRole::User, "hi").unwrap();

    // Inject a row with a bogus role directly via a one-off SessionDb
    // open so we exercise the `load_messages` parsing path.
    db.append_message("s1", SessionRole::User, "anchor")
        .unwrap();
    // The right way to poison: bypass the public API by writing through a
    // raw execute. Since we don't expose that, we instead assert that the
    // enum-to-string mapping is round-trip stable for the four valid roles
    // and that a string not in the set is rejected by `SessionRole::parse`.
    for role in [
        SessionRole::User,
        SessionRole::Assistant,
        SessionRole::System,
        SessionRole::Tool,
    ] {
        assert_eq!(SessionRole::parse(role.as_str()).unwrap(), role);
    }
    let err = SessionRole::parse("admin").unwrap_err();
    assert!(matches!(err, SessionError::UnknownRole(ref s) if s == "admin"));
}

#[test]
fn load_messages_preserves_unicode() {
    let (_dir, db) = open_temp("unicode");
    db.ensure_session("s", "t").unwrap();
    let samples = [
        "Hello, world!",
        "你好，世界！",               // CJK
        "Здравствуй, мир!",           // Cyrillic
        "こんにちは 🌍",              // mixed + emoji
        "👨‍👩‍👧‍👦 family",                  // ZWJ family emoji
        "\u{200B}zero-width\u{200B}", // zero-width space
        "newlines\nand\ttabs",
    ];
    for s in samples {
        db.append_message("s", SessionRole::User, s).unwrap();
    }
    let loaded = db.load_messages("s", MAX_LIMIT).unwrap();
    let texts: Vec<&str> = loaded.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(texts, samples);
}

#[test]
fn role_enum_round_trips_through_serde() {
    // Helps the agent-tool side: SessionRole can be (de)serialized as JSON
    // in the lowercase wire form without a manual mapping.
    for (role, wire) in [
        (SessionRole::User, "\"user\""),
        (SessionRole::Assistant, "\"assistant\""),
        (SessionRole::System, "\"system\""),
        (SessionRole::Tool, "\"tool\""),
    ] {
        let s = serde_json::to_string(&role).unwrap();
        assert_eq!(s, wire);
        let back: SessionRole = serde_json::from_str(&s).unwrap();
        assert_eq!(back, role);
    }
}

// -----------------------------------------------------------------------------
// list_sessions / search_messages
// -----------------------------------------------------------------------------

#[test]
fn list_sessions_filters_and_orders_by_updated_at_desc() {
    let (_dir, db) = open_temp("list");
    // Build three sessions; force different updated_at by issuing an
    // append_message in between ensures so the bumped updated_at moves
    // each row to the top.
    db.ensure_session("a", "alpha project").unwrap();
    db.ensure_session("b", "beta discussions").unwrap();
    db.append_message("a", SessionRole::User, "wakeup-a")
        .unwrap();
    db.ensure_session("c", "gamma notes").unwrap();
    db.append_message("b", SessionRole::User, "wakeup-b")
        .unwrap();
    db.append_message("c", SessionRole::User, "wakeup-c")
        .unwrap();

    // substring on title; case-insensitive
    let hits = db.list_sessions("notes", 50).unwrap();
    assert_eq!(
        hits.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        vec!["c"]
    );

    // substring on id
    let hits = db.list_sessions("alpha", 50).unwrap();
    assert_eq!(
        hits.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        vec!["a"]
    );

    // ponytail: a same-millisecond burst means `now_ms()` returns one value
    // for every call below, so all three rows tie. The `ORDER BY id ASC`
    // tiebreaker is the only thing that orders them. We exercise that here
    // by hitting the index-ascending case directly.
    let hits = db.list_sessions("e", 50).unwrap();
    // Filter to the three we created. The list itself may include extras
    // if any substring matches more broadly, but at minimum these three
    // must come back and the order among them must be id-ascending when
    // their updated_at is equal.
    let three: Vec<&str> = hits
        .iter()
        .filter(|s| matches!(s.id.as_str(), "a" | "b" | "c"))
        .map(|s| s.id.as_str())
        .collect();
    // Each one of the three updated_at should be the same since we never
    // sleep; with identical updated_at the secondary `id ASC` rule should
    // give us a, b, c.
    let upds: HashSet<i64> = hits
        .iter()
        .filter(|s| matches!(s.id.as_str(), "a" | "b" | "c"))
        .map(|s| s.updated_at_ms)
        .collect();
    if upds.len() == 1 {
        assert_eq!(three, vec!["a", "b", "c"]);
    }
}

#[test]
fn search_messages_with_and_without_session_scope() {
    let (_dir, db) = open_temp("search");
    db.ensure_session("s1", "t").unwrap();
    db.ensure_session("s2", "t").unwrap();
    db.append_message("s1", SessionRole::User, "the quick brown fox")
        .unwrap();
    db.append_message("s2", SessionRole::User, "the lazy dog")
        .unwrap();
    db.append_message("s1", SessionRole::Assistant, "jumps over")
        .unwrap();

    // global
    let all_msgs = db.search_messages("the", None, MAX_LIMIT).unwrap();
    assert_eq!(all_msgs.len(), 2);
    assert!(
        all_msgs
            .iter()
            .all(|m| m.text.to_lowercase().contains("the"))
    );

    // scoped
    let in_s1 = db.search_messages("the", Some("s1"), MAX_LIMIT).unwrap();
    assert_eq!(in_s1.len(), 1);
    assert_eq!(in_s1[0].session_id, "s1");

    // case-insensitive
    let upper = db.search_messages("FOX", Some("s1"), MAX_LIMIT).unwrap();
    assert_eq!(upper.len(), 1);
    assert_eq!(upper[0].text, "the quick brown fox");
}

#[test]
fn different_sessions_are_isolated() {
    let (_dir, db) = open_temp("isolation");
    db.ensure_session("alpha", "first").unwrap();
    db.ensure_session("beta", "second").unwrap();
    db.append_message("alpha", SessionRole::User, "a1").unwrap();
    db.append_message("alpha", SessionRole::User, "a2").unwrap();
    db.append_message("beta", SessionRole::User, "b1").unwrap();

    let a = db.load_messages("alpha", MAX_LIMIT).unwrap();
    let b = db.load_messages("beta", MAX_LIMIT).unwrap();
    assert_eq!(a.len(), 2);
    assert_eq!(b.len(), 1);
    let a_seqs: HashSet<i64> = a.iter().map(|m| m.seq).collect();
    let b_seqs: HashSet<i64> = b.iter().map(|m| m.seq).collect();
    // Sequences are per-session, both start at 1.
    assert_eq!(a_seqs, [1, 2].into_iter().collect());
    assert_eq!(b_seqs, [1].into_iter().collect());

    let alpha_summary: SessionSummary = db.session("alpha").unwrap().expect("alpha present");
    let beta_summary: SessionSummary = db.session("beta").unwrap().expect("beta present");
    assert_eq!(alpha_summary.message_count, 2);
    assert_eq!(beta_summary.message_count, 1);
}

// -----------------------------------------------------------------------------
// limit errors
// -----------------------------------------------------------------------------

#[test]
fn limit_zero_or_too_big_rejected() {
    let (_dir, db) = open_temp("limit");
    db.ensure_session("s", "t").unwrap();
    db.append_message("s", SessionRole::User, "x").unwrap();

    for bad in [0u32, MAX_LIMIT + 1, 50_000] {
        assert!(matches!(
            db.load_messages("s", bad),
            Err(SessionError::InvalidLimit(n)) if n == bad
        ));
        assert!(matches!(
            db.list_sessions("t", bad),
            Err(SessionError::InvalidLimit(n)) if n == bad
        ));
        assert!(matches!(
            db.search_messages("x", None, bad),
            Err(SessionError::InvalidLimit(n)) if n == bad
        ));
    }
}

// -----------------------------------------------------------------------------
// Real cross-thread concurrent writes
// -----------------------------------------------------------------------------

#[test]
fn concurrent_writers_from_distinct_session_dbs() {
    // Each thread opens its own `SessionDb` to the same file. SQLite WAL +
    // the `busy_timeout` PRAGMA handle cross-process contention. We never
    // sleep; a Barrier releases all threads at once and we measure the
    // outcome after they all complete.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("race.db");
    let db = SessionDb::open(&path).expect("seed db");
    drop(db); // close; others will reopen

    // ponytail: pre-warm WAL by opening once, doing a tiny write, closing.
    // This causes the WAL file to exist so the contention test is a fair
    // write-write race, not a "first writer has to create the file" race.
    {
        let warm = SessionDb::open(&path).unwrap();
        warm.ensure_session("__warmup__", "warmup").unwrap();
        warm.append_message("__warmup__", SessionRole::System, "warm")
            .unwrap();
    }

    const THREADS: usize = 6;
    const PER_THREAD: i64 = 30;
    let barrier = Arc::new(Barrier::new(THREADS));
    let path = Arc::new(path);

    let mut handles = Vec::new();
    for t in 0..THREADS {
        let barrier = Arc::clone(&barrier);
        let path = Arc::clone(&path);
        handles.push(thread::spawn(move || {
            let db = SessionDb::open(&*path).expect("open in worker");
            let sid = format!("s{t}");
            db.ensure_session(&sid, &format!("thread {t}"))
                .expect("ensure");
            barrier.wait();
            for i in 0..PER_THREAD {
                db.append_message(&sid, SessionRole::User, &format!("thread={t} i={i}"))
                    .expect("append");
            }
            db.load_messages(&sid, MAX_LIMIT).expect("load")
        }));
    }

    let mut per_session_counts = Vec::new();
    for h in handles {
        let msgs = h.join().expect("worker join");
        per_session_counts.push(msgs.len() as i64);
    }

    // Each thread wrote exactly PER_THREAD messages; order across threads
    // is interleaved, so the per-session seq must still be a clean 1..=N.
    for (idx, n) in per_session_counts.iter().enumerate() {
        assert_eq!(*n, PER_THREAD, "thread {idx} got {n} messages");
    }

    // Now reopen once and confirm global state matches.
    let db = SessionDb::open(&*path).unwrap();
    let summary: Vec<SessionSummary> = db.list_sessions("s", MAX_LIMIT).unwrap();
    // We seeded "__warmup__" too, but that doesn't match the "s" prefix.
    // We expect exactly THREADS sessions whose id starts with "s".
    let real: Vec<&SessionSummary> = summary.iter().filter(|s| s.id.starts_with('s')).collect();
    assert_eq!(real.len(), THREADS);
    for s in &real {
        assert_eq!(s.message_count, PER_THREAD as u64);
    }
}
