//! Q1 attachments: `messages.attachments` write/read round-trip through the
//! public `SessionDb` API.
//!
//! Pins the contracts the attachment pipeline leans on:
//!
//! 1. a message written with attachments reads back with `name`,
//!    `media_type` and `data` intact (order preserved);
//! 2. a message written without attachments reads back as an empty vec;
//! 3. a corrupted `attachments` column degrades to an empty vec instead of
//!    failing the whole session read;
//! 4. a file written by a pre-Q1 build (no `attachments` column) gains the
//!    column on first open, keeps its rows (reading them as "no
//!    attachments"), and accepts new attached messages — the migration's
//!    upgrade-day path.

use session::{Attachment, SessionDb, SessionRole};
use tempfile::tempdir;

fn attach(name: &str, media_type: &str, data: &str) -> Attachment {
    Attachment {
        name: name.into(),
        media_type: media_type.into(),
        data: data.into(),
    }
}

#[test]
fn message_with_attachments_round_trips() {
    let dir = tempdir().expect("temp dir");
    let db = SessionDb::open(dir.path().join("sessions.db")).expect("open db");

    let (seq, _) = db
        .append_message(
            "att-1",
            SessionRole::User,
            "看图说话",
            &[
                attach("screenshot.png", "image/png", "aWFsb25lPGRhdGE+"),
                attach("chart.webp", "image/webp", "UElFWHByb2ZpbGU="),
            ],
        )
        .expect("append message with attachments");
    assert_eq!(seq, 1, "first message of a session is seq 1");

    let rows = db.load_messages("att-1", 10).expect("load messages");
    assert_eq!(rows.len(), 1, "exactly one message was written");
    let msg = &rows[0];
    assert_eq!(msg.text, "看图说话", "text is untouched by attachments");
    assert_eq!(
        msg.attachments.len(),
        2,
        "both attachments survive the round trip"
    );
    assert_eq!(msg.attachments[0].name, "screenshot.png");
    assert_eq!(msg.attachments[0].media_type, "image/png");
    assert_eq!(msg.attachments[0].data, "aWFsb25lPGRhdGE+");
    assert_eq!(msg.attachments[1].name, "chart.webp");
    assert_eq!(msg.attachments[1].media_type, "image/webp");
    assert_eq!(msg.attachments[1].data, "UElFWHByb2ZpbGU=");
}

#[test]
fn message_without_attachments_reads_back_empty() {
    let dir = tempdir().expect("temp dir");
    let db = SessionDb::open(dir.path().join("sessions.db")).expect("open db");

    db.append_message("att-2", SessionRole::User, "纯文本", &[])
        .expect("append bare message");

    let rows = db.load_messages("att-2", 10).expect("load messages");
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0].attachments.is_empty(),
        "a message written without attachments must read back as none"
    );

    // search_messages returns the same `SessionMessage` shape and must agree.
    let found = db
        .search_messages("纯文本", None, 10)
        .expect("search messages");
    assert_eq!(found.len(), 1);
    assert!(
        found[0].attachments.is_empty(),
        "search path reads attachments too"
    );
}

#[test]
fn corrupted_attachments_column_degrades_to_empty() {
    let dir = tempdir().expect("temp dir");
    let db = SessionDb::open(dir.path().join("sessions.db")).expect("open db");
    let db_path = db.path().to_path_buf();

    db.append_message(
        "att-3",
        SessionRole::User,
        "坏数据",
        &[attach("a.png", "image/png", "aW1n")],
    )
    .expect("append with attachments first");

    // Bypass the public API and smash the column: the reader must degrade
    // to "no attachments" instead of erroring the whole session read. Same
    // raw-connection bootstrap shape as `tests/lineage.rs`.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("corrupt runtime");
    runtime.block_on(async {
        let raw = libsql::Builder::new_local(&db_path)
            .build()
            .await
            .expect("open raw db");
        let conn = raw.connect().expect("connect raw db");
        conn.execute(
            "UPDATE messages SET attachments = ?1 WHERE session_id = ?2",
            libsql::params!["{not-json", "att-3"],
        )
        .await
        .expect("corrupt the attachments column");
    });

    let rows = db
        .load_messages("att-3", 10)
        .expect("read survives corruption");
    assert_eq!(rows.len(), 1, "the row itself must survive");
    assert_eq!(
        rows[0].text, "坏数据",
        "text is unaffected by a bad attachments column"
    );
    assert!(
        rows[0].attachments.is_empty(),
        "malformed JSON degrades to an empty vec, not an error"
    );
}

/// Build a database file in the pre-Q1 schema: `messages` without an
/// `attachments` column, one legacy row already in it. A file created by
/// current code gets the column from `CREATE TABLE`, so this is the only
/// way to reach the migration's `ALTER` branch — the same trick
/// `tests/lineage.rs` uses for `parent_id`.
fn legacy_file(db_path: &std::path::Path) {
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
             CREATE TABLE messages (
                 session_id TEXT NOT NULL,
                 seq INTEGER NOT NULL,
                 role TEXT NOT NULL,
                 text TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 PRIMARY KEY (session_id, seq),
                 FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
             );
             INSERT INTO sessions (id, title, created_at, updated_at, turn_log)
             VALUES ('legacy-row', '老会话', 1, 1, '');
             INSERT INTO messages (session_id, seq, role, text, created_at)
             VALUES ('legacy-row', 1, 'user', '旧消息', 2);",
        )
        .await
        .expect("write pre-Q1 schema");
    });
}

/// A pre-Q1 file must open, keep its legacy row (reading it as "no
/// attachments"), and accept new attached messages — the migration exists
/// for exactly this upgrade-day path.
#[test]
fn legacy_file_without_column_is_migrated() {
    let dir = tempdir().expect("temp dir");
    let db_path = dir.path().join("sessions.db");
    legacy_file(&db_path);

    let db = SessionDb::open(&db_path).expect("open legacy file with current code");

    let rows = db
        .load_messages("legacy-row", 10)
        .expect("load legacy message");
    assert_eq!(rows.len(), 1, "the pre-existing row survives the migration");
    assert!(
        rows[0].attachments.is_empty(),
        "a row written before the column existed reads back as no attachments"
    );

    // The column is now live: a new message can carry attachments.
    db.append_message(
        "legacy-row",
        SessionRole::Assistant,
        "迁移后新消息",
        &[attach("n.png", "image/png", "aW1n")],
    )
    .expect("append after migration");
    let rows = db
        .load_messages("legacy-row", 10)
        .expect("reload after append");
    assert_eq!(rows.len(), 2);
    assert!(
        rows[0].attachments.is_empty(),
        "legacy row stays attachment-free"
    );
    assert_eq!(rows[1].attachments.len(), 1);
    assert_eq!(rows[1].attachments[0].data, "aW1n");
}
