//! G6 — crash repair wiring (gap C).
//!
//! A daemon killed mid-run leaves a `TurnStart` with no `TurnEnd` in the
//! persisted turn log. `Daemon::start` runs `interrupted_run_closers` once,
//! after the SessionDb is open and before the socket is bound, so the
//! half-open run is closed as `aborted` before any client can connect and
//! see it as live. Today that function was only ever called from
//! `session/tests/turn_repair.rs`; this is the production path.

use daemon::{Daemon, DaemonConfig};
use session::{SessionDb, TurnRecord};
use tempfile::tempdir;

fn cfg(dir: &std::path::Path, socket: &str, db: &std::path::Path) -> DaemonConfig {
    DaemonConfig {
        socket_path: Some(dir.join(socket)),
        omp_path: "omp".into(),
        session_db_path: Some(db.to_path_buf()),
        orbit_model: None,
        // Scope instruction discovery to the temp dir.
        cwd: dir.to_path_buf(),
        max_turns: 64,
        mcp_servers: Vec::new(),
        llm_fallbacks: Vec::new(),
    }
}

/// A session log with one run that started and never ended — the exact shape
/// a `kill -9` between `prompt` and its response leaves behind.
#[test]
fn startup_repair_closes_interrupted_run_as_aborted() {
    let dir = tempdir().expect("temp dir");
    let db_path = dir.path().join("sessions.db");

    // The crashed previous process: run started, no closer written.
    let db = SessionDb::open(&db_path).expect("open db");
    db.ensure_session("s1", "unfinished").expect("session row");
    db.append_turn_log(
        "s1",
        &[TurnRecord::TurnStart {
            run_id: "r1".into(),
            ts_ms: 1_000,
        }],
    )
    .expect("record start");
    assert_eq!(db.load_turn_log("s1").unwrap().len(), 1);
    drop(db);

    let daemon = Daemon::start(cfg(dir.path(), "daemon.sock", &db_path)).expect("daemon start");

    let records = daemon
        .sessions()
        .load_turn_log("s1")
        .expect("read repaired log");
    assert_eq!(
        records.len(),
        2,
        "the interrupted run gained exactly one closer"
    );
    match &records[1] {
        TurnRecord::TurnEnd { run_id, status, .. } => {
            assert_eq!(run_id, "r1", "the closer is for the open run");
            assert_eq!(status, session::ABORTED, "the closer marks the run aborted");
        }
        other => panic!("expected a TurnEnd closer, got {other:?}"),
    }
    // The pre-existing record is untouched: repair only ever continues a log.
    assert!(matches!(
        &records[0],
        TurnRecord::TurnStart { run_id, ts_ms } if run_id == "r1" && *ts_ms == 1_000
    ));
}

/// Repair is idempotent: a log the previous start already repaired has no
/// open runs, so the next start appends nothing.
#[test]
fn repair_is_idempotent_across_restarts() {
    let dir = tempdir().expect("temp dir");
    let db_path = dir.path().join("sessions.db");

    let db = SessionDb::open(&db_path).expect("open db");
    db.ensure_session("s1", "crashed once")
        .expect("session row");
    db.append_turn_log(
        "s1",
        &[TurnRecord::TurnStart {
            run_id: "r1".into(),
            ts_ms: 1_000,
        }],
    )
    .expect("record start");
    drop(db);

    // First start: one open run -> one closer.
    let first = Daemon::start(cfg(dir.path(), "daemon.sock", &db_path)).expect("first start");
    assert_eq!(first.sessions().load_turn_log("s1").unwrap().len(), 2);
    drop(first);

    // Second start over the same DB: the log is balanced, nothing is added.
    // A distinct socket keeps the bind off the first daemon's cleanup path.
    let second = Daemon::start(cfg(dir.path(), "daemon2.sock", &db_path)).expect("second start");
    assert_eq!(
        second.sessions().load_turn_log("s1").unwrap().len(),
        2,
        "a repaired log must not grow on the next start"
    );
}

/// A session whose runs all closed cleanly is left alone — the repair pass
/// is a no-op on a healthy database.
#[test]
fn clean_log_is_left_untouched() {
    let dir = tempdir().expect("temp dir");
    let db_path = dir.path().join("sessions.db");

    let db = SessionDb::open(&db_path).expect("open db");
    db.ensure_session("s1", "clean").expect("session row");
    db.append_turn_log(
        "s1",
        &[
            TurnRecord::TurnStart {
                run_id: "r1".into(),
                ts_ms: 1_000,
            },
            TurnRecord::TurnEnd {
                run_id: "r1".into(),
                ts_ms: 2_000,
                status: "ok".into(),
            },
        ],
    )
    .expect("record clean run");
    drop(db);

    let daemon = Daemon::start(cfg(dir.path(), "daemon.sock", &db_path)).expect("daemon start");
    let records = daemon.sessions().load_turn_log("s1").unwrap();
    assert_eq!(records.len(), 2, "a clean log is unchanged");
}
