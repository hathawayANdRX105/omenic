//! `task.list` — the kanban board's read link to the CLI task store.
//!
//! These tests start a real daemon on a temp Unix socket and read the
//! `tasks.jsonl` that `oi task add` / `task done` write, so the whole
//! socket → dispatch → `task::Store` → jsonl chain runs end to end — no
//! mocks, no in-memory shortcuts.  They are the smoke evidence for the
//! read side of the task board (the UI wiring lands in a follow-up PR).

use std::path::Path;

use daemon::protocol::Command;
use daemon::{Daemon, DaemonClient, DaemonConfig};
use serde_json::json;
use task::store::Store;
use task::{Task, TaskKind, TaskStatus};
use tempfile::TempDir;

/// Start a daemon whose `data_dir` is `dir/data` — the same place the CLI
/// writes `tasks.jsonl`, so the store the handler opens is a real one.
fn start_daemon(dir: &Path) -> Daemon {
    Daemon::start(DaemonConfig {
        socket_path: Some(dir.join("daemon.sock")),
        // No worker is spawned by these tests — `task.list` never touches
        // it, so the path is deliberately a binary that does not exist.
        omp_path: "omp-not-used-by-task-list-tests".to_string(),
        session_db_path: Some(dir.join("sessions.db")),
        orbit_model: None,
        cwd: dir.to_path_buf(),
        max_turns: 64,
        data_dir: dir.join("data"),
        ..Default::default()
    })
    .expect("daemon start")
}

/// One task appended through the store the CLI uses (flock + fsync), with
/// `created_at` fixed and only `updated_at` varying per test case.
fn seed_task(data_dir: &Path, id: &str, title: &str, updated_at: &str) {
    let task = Task {
        id: id.to_string(),
        title: title.to_string(),
        kind: TaskKind::Task,
        status: TaskStatus::Open,
        attempts: 0,
        priority: 2,
        parent: None,
        deps: Vec::new(),
        description: String::new(),
        acceptance: String::new(),
        created_at: "2026-09-20T00:00:00Z".to_string(),
        updated_at: updated_at.to_string(),
    };
    Store::new(data_dir)
        .append(&task)
        .unwrap_or_else(|e| panic!("seed task {id}: {e}"));
}

/// Ask the daemon for the task list, typed so a field-shape regression in
/// the response fails here rather than in the UI later.
fn task_list(client: &DaemonClient, limit: Option<u32>) -> Vec<Task> {
    let params = match limit {
        Some(n) => json!({ "limit": n }),
        None => json!({}),
    };
    client
        .call::<Vec<Task>>(Command::TaskList, params)
        .expect("task.list round trip")
}

/// Ordering is `updated_at` descending, not id (title-slug) order.  The
/// seeds are deliberately slugged so the two orders disagree: alphabetically
/// `alpha` < `bravo` < `charlie`, but by update time `bravo` is newest and
/// `alpha` oldest.  Goes red if the handler sorts ascending, sorts by id,
/// or forgets the sort entirely (the store itself returns id order).
#[test]
fn task_list_returns_recently_updated_first() {
    let dir = TempDir::new().expect("temp dir");
    let data_dir = dir.path().join("data");
    seed_task(&data_dir, "alpha", "Alpha task", "2026-09-01T10:00:00Z");
    seed_task(&data_dir, "bravo", "Bravo task", "2026-09-03T10:00:00Z");
    seed_task(&data_dir, "charlie", "Charlie task", "2026-09-02T10:00:00Z");

    let daemon = start_daemon(dir.path());
    let client = DaemonClient::connect_to(daemon.socket_addr().path());

    let tasks = task_list(&client, None);
    let ids: Vec<&str> = tasks.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["bravo", "charlie", "alpha"],
        "task.list must be newest-first by updated_at, not id order"
    );
}

/// `limit: 0` is a legal empty page, not an error; a missing `limit` means
/// the default page (50), which returns everything when the store is
/// smaller than that; an explicit `limit` truncates from the newest end.
/// Goes red if `limit` is treated as required (the bare `{}` call fails),
/// if the default is 0 (the un-paginated call comes back empty), or if the
/// truncation path returns the whole store.
#[test]
fn task_list_limit_zero_and_default() {
    let dir = TempDir::new().expect("temp dir");
    let data_dir = dir.path().join("data");
    seed_task(&data_dir, "alpha", "Alpha task", "2026-09-01T10:00:00Z");
    seed_task(&data_dir, "bravo", "Bravo task", "2026-09-03T10:00:00Z");
    seed_task(&data_dir, "charlie", "Charlie task", "2026-09-02T10:00:00Z");

    let daemon = start_daemon(dir.path());
    let client = DaemonClient::connect_to(daemon.socket_addr().path());

    assert!(
        task_list(&client, Some(0)).is_empty(),
        "limit 0 must be an empty page, not an error"
    );

    let one = task_list(&client, Some(1));
    assert_eq!(
        one.len(),
        1,
        "limit must truncate from the newest end, not pass the store through"
    );
    assert_eq!(one[0].id, "bravo", "the kept row is the most recent one");

    assert_eq!(
        task_list(&client, None).len(),
        3,
        "a store smaller than the default page must come back in full"
    );
}

/// A data dir with no `tasks.jsonl` at all (the CLI was never run) answers
/// an empty list, not an error — the board renders an empty state on a
/// fresh install.  Goes red if a missing file is propagated as an error
/// response instead of `[]`.
#[test]
fn task_list_empty_when_no_store() {
    let dir = TempDir::new().expect("temp dir");
    // `data` is never created: the handler's `Store::new` makes it, and
    // `load_all` treats a absent file as no records.
    let daemon = start_daemon(dir.path());
    let client = DaemonClient::connect_to(daemon.socket_addr().path());

    let tasks = task_list(&client, None);
    assert!(
        tasks.is_empty(),
        "an unseen data dir must answer an empty list, not fail"
    );
}
