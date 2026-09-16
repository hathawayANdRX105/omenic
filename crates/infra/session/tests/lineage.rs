//! G7-A1 — session lineage: the `parent_id` column and its wire round-trip.
//!
//! The parent edge is the data foundation for 5.3/5.5 grouping. These tests
//! pin three contracts the grouping layer leans on:
//!
//! 1. the column migration is idempotent (every `open` re-runs it and a
//!    file that already has the column costs one `PRAGMA table_info` and no
//!    DDL, never a duplicate-column error);
//! 2. a file written by a pre-G7 build — no `parent_id` column at all —
//!    gains the column on first open, keeps its rows, and reads them as
//!    roots (the upgrade-day path; a fresh file gets the column from
//!    `CREATE TABLE` and never exercises the `ALTER`);
//! 3. a parent passed at creation survives a `list` round-trip;
//! 4. a session created without a parent is a root (`None`), and a blank
//!    parent string is treated the same as none;
//! 5. a multi-level chain (a → b → c) is fully recoverable from `list`.

use session::SessionDb;
use tempfile::tempdir;

/// Build a database file in the pre-G7 schema (no `parent_id` column) with
/// one row already in it. A file created by current code gets the column from
/// `CREATE TABLE`, so this is the only way to reach the migration's `ALTER`
/// branch — the upgrade-day path the migration exists for.
fn legacy_file(db_path: &std::path::Path) {
    // Same runtime shape SessionDb::open builds for itself: a dedicated
    // current-thread runtime owned by this call, driven to completion here.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime");
    runtime.block_on(async {
        let db = libsql::Builder::new_local(db_path)
            .build()
            .await
            .expect("open legacy file");
        let conn = db.connect().expect("connect legacy file");
        conn.execute_batch(
            "CREATE TABLE sessions (
                 id TEXT PRIMARY KEY,
                 title TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 turn_log TEXT NOT NULL DEFAULT ''
             );
             INSERT INTO sessions (id, title, created_at, updated_at, turn_log)
             VALUES ('legacy-row', '老库', 1, 2, '');",
        )
        .await
        .expect("write pre-G7 schema");
    });
}

/// A file written by an omenic build predating `parent_id` must gain the
/// column on first open, keep its existing rows, and read them as roots.
/// This is the migration's reason for existing; the idempotency test above
/// only covers the already-migrated path.
#[test]
fn legacy_file_without_column_is_migrated() {
    let dir = tempdir().expect("temp dir");
    let db_path = dir.path().join("sessions.db");
    legacy_file(&db_path);

    // First open by current code: init_database → apply_parent_id_column sees
    // the missing column and ALTERs it in.
    let db = SessionDb::open(&db_path).expect("open legacy file with current code");

    let rows = db
        .list_sessions("legacy-row", 10)
        .expect("list after migration");
    assert_eq!(rows.len(), 1, "the pre-existing row survives the migration");
    assert_eq!(rows[0].id, "legacy-row");
    assert!(
        rows[0].parent_id.is_none(),
        "the added column defaults to NULL, so a legacy row is a root"
    );

    // The column is now live: a child can point at the migrated legacy row.
    let child = db
        .ensure_session_with_parent("new-child", "新子会话", Some("legacy-row"))
        .expect("create child under the legacy row");
    assert_eq!(child.parent_id.as_deref(), Some("legacy-row"));
}

/// Re-open the same file and confirm the already-migrated column costs
/// nothing — no duplicate-column error, and the lineage edge written by the
/// first handle is still readable through the second.
#[test]
fn parent_id_column_added_idempotently() {
    let dir = tempdir().expect("temp dir");
    let db_path = dir.path().join("sessions.db");

    let db = SessionDb::open(&db_path).expect("open db (first)");
    // Writing a parent exercises the column end to end: the INSERT would fail
    // outright if the migration had not run.
    let row = db
        .ensure_session_with_parent("child", "title", Some("parent"))
        .expect("create with parent");
    assert_eq!(row.parent_id.as_deref(), Some("parent"));
    drop(db);

    // Second open re-runs init_database → apply_parent_id_column, which must
    // see the column and early-return rather than ALTER again.
    let db = SessionDb::open(&db_path).expect("open db (second, already migrated)");
    let again = db
        .ensure_session("child", "title")
        .expect("ensure existing");
    // Parent is preserved on an existing row (ensure never clobbers lineage).
    assert_eq!(again.parent_id.as_deref(), Some("parent"));
}

/// A parent passed to create comes back identically from `list_sessions`.
#[test]
fn create_with_parent_round_trips() {
    let dir = tempdir().expect("temp dir");
    let db = SessionDb::open(&dir.path().join("sessions.db")).expect("open db");

    let row = db
        .create_session_with_parent("rt-1", "往返测试", Some("rt-parent"))
        .expect("create with parent");
    assert_eq!(row.id, "rt-1");
    assert_eq!(row.parent_id.as_deref(), Some("rt-parent"));

    let listed = db.list_sessions("rt-1", 10).expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "rt-1");
    assert_eq!(listed[0].parent_id.as_deref(), Some("rt-parent"));
}

/// No parent → root. A blank parent is normalized to root too, so a stray
/// empty string can never masquerade as a lineage edge.
#[test]
fn orphans_have_no_parent() {
    let dir = tempdir().expect("temp dir");
    let db = SessionDb::open(&dir.path().join("sessions.db")).expect("open db");

    let explicit_none = db
        .ensure_session_with_parent("solo", "无父", None)
        .expect("create with explicit None");
    assert!(explicit_none.parent_id.is_none());

    // The legacy 2-arg shape (unchanged signature) is still a root.
    let legacy = db.ensure_session("legacy", "老路径").expect("ensure 2-arg");
    assert!(legacy.parent_id.is_none());

    // A blank parent is the same as none.
    let blank = db
        .ensure_session_with_parent("blank", "空串", Some("   "))
        .expect("create with blank parent");
    assert!(blank.parent_id.is_none());

    let listed = db.list_sessions("solo", 10).expect("list");
    assert_eq!(listed.len(), 1);
    assert!(listed[0].parent_id.is_none());
}

/// A three-level chain a → b → c is fully recoverable from one `list` call:
/// the grouping layer walks `parent_id` from any node back to its root.
#[test]
fn lineage_chain_queries() {
    let dir = tempdir().expect("temp dir");
    let db = SessionDb::open(&dir.path().join("sessions.db")).expect("open db");

    db.ensure_session("a", "chain-a").expect("root");
    db.ensure_session_with_parent("b", "chain-b", Some("a"))
        .expect("child");
    db.ensure_session_with_parent("c", "chain-c", Some("b"))
        .expect("grandchild");

    let rows = db.list_sessions("chain-", 10).expect("list");
    assert_eq!(rows.len(), 3, "all three chain sessions match");

    let by_id: std::collections::HashMap<&str, Option<&str>> = rows
        .iter()
        .map(|r| (r.id.as_str(), r.parent_id.as_deref()))
        .collect();

    // c → b → a → root.
    assert_eq!(by_id.get("c").copied().flatten(), Some("b"));
    assert_eq!(by_id.get("b").copied().flatten(), Some("a"));
    assert!(by_id.get("a").copied().flatten().is_none());

    // A single-hop lookup agrees with the list view.
    assert_eq!(
        db.session("c")
            .expect("get")
            .expect("present")
            .parent_id
            .as_deref(),
        Some("b")
    );
}
