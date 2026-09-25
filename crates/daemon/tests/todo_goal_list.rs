//! `todo.list` / `goal.list` — the board's read link to the CLI todo/goal
//! stores.
//!
//! These tests start a real daemon on a temp Unix socket and read the
//! `todos.jsonl` / `goals.jsonl` that the CLI (and, from F3, the model's
//! todo/goal tools) append, so the whole socket → dispatch → `store::Store`
//! → jsonl chain runs end to end — no mocks, no in-memory shortcuts.  They
//! are the smoke evidence for the read side of the todo/goal board (the UI
//! wiring lands in a follow-up PR).

use std::path::Path;

use daemon::protocol::Command;
use daemon::{Daemon, DaemonClient, DaemonConfig};
use serde_json::json;
use store::goal::{Goal, GoalStatus};
use store::store::Store;
use store::todo::{Todo, TodoStatus};
use tempfile::TempDir;

/// Start a daemon whose `data_dir` is `dir/data` — the same place the CLI
/// writes `todos.jsonl` / `goals.jsonl`, so the store the handler opens is
/// a real one.
fn start_daemon(dir: &Path) -> Daemon {
    Daemon::start(DaemonConfig {
        socket_path: Some(dir.join("daemon.sock")),
        // No worker is spawned by these tests — the list RPCs never touch
        // it, so the path is deliberately a binary that does not exist.
        omp_path: "omp-not-used-by-todo-goal-list-tests".to_string(),
        session_db_path: Some(dir.join("sessions.db")),
        orbit_model: None,
        cwd: dir.to_path_buf(),
        max_turns: 64,
        data_dir: dir.join("data"),
        ..Default::default()
    })
    .expect("daemon start")
}

/// One todo appended through the store the CLI uses (flock + fsync), with
/// `created_at` fixed and only `updated_at` varying per test case.
fn seed_todo(data_dir: &Path, id: &str, updated_at: &str) {
    let todo = Todo {
        id: id.to_string(),
        title: id.to_string(),
        status: TodoStatus::Open,
        created_at: "2026-09-20T00:00:00Z".to_string(),
        updated_at: updated_at.to_string(),
        note: None,
    };
    Store::new(data_dir)
        .append_todo(&todo)
        .unwrap_or_else(|e| panic!("seed todo {id}: {e}"));
}

/// One goal appended through the store the CLI uses, with `created_at`
/// fixed and only `updated_at` varying per test case.
fn seed_goal(data_dir: &Path, id: &str, updated_at: &str) {
    let goal = Goal {
        id: id.to_string(),
        title: id.to_string(),
        status: GoalStatus::Active,
        todo_ids: Vec::new(),
        created_at: "2026-09-20T00:00:00Z".to_string(),
        updated_at: updated_at.to_string(),
    };
    Store::new(data_dir)
        .append_goal(&goal)
        .unwrap_or_else(|e| panic!("seed goal {id}: {e}"));
}

/// Ask the daemon for the todo list, typed so a field-shape regression in
/// the response fails here rather than in the UI later.
fn todo_list(client: &DaemonClient, limit: Option<u32>) -> Vec<Todo> {
    let params = match limit {
        Some(n) => json!({ "limit": n }),
        None => json!({}),
    };
    client
        .call::<Vec<Todo>>(Command::TodoList, params)
        .expect("todo.list round trip")
}

/// Ask the daemon for the goal list, typed the same way.
fn goal_list(client: &DaemonClient, limit: Option<u32>) -> Vec<Goal> {
    let params = match limit {
        Some(n) => json!({ "limit": n }),
        None => json!({}),
    };
    client
        .call::<Vec<Goal>>(Command::GoalList, params)
        .expect("goal.list round trip")
}

/// Ordering is `updated_at` descending, not id (title-slug) order.  The
/// seeds are deliberately slugged so the two orders disagree: alphabetically
/// `alpha` < `bravo` < `charlie`, but by update time `bravo` is newest and
/// `alpha` oldest.  Goes red if the handler sorts ascending, sorts by id,
/// or forgets the sort entirely (the store itself returns id order).
#[test]
fn todo_list_returns_recently_updated_first() {
    let dir = TempDir::new().expect("temp dir");
    let data_dir = dir.path().join("data");
    seed_todo(&data_dir, "alpha", "2026-09-01T10:00:00Z");
    seed_todo(&data_dir, "bravo", "2026-09-03T10:00:00Z");
    seed_todo(&data_dir, "charlie", "2026-09-02T10:00:00Z");

    let daemon = start_daemon(dir.path());
    let client = DaemonClient::connect_to(daemon.socket_addr().path());

    let todos = todo_list(&client, None);
    let ids: Vec<&str> = todos.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["bravo", "charlie", "alpha"],
        "todo.list must be newest-first by updated_at, not id order"
    );
}

/// `limit: 0` is a legal empty page, not an error; a missing `limit` means
/// the default page (50), which returns everything when the store is
/// smaller than that; an explicit `limit` truncates from the newest end.
/// Goes red if `limit` is treated as required (the bare `{}` call fails),
/// if the default is 0 (the un-paginated call comes back empty), or if the
/// truncation path returns the whole store.
#[test]
fn goal_list_limit_zero_and_default() {
    let dir = TempDir::new().expect("temp dir");
    let data_dir = dir.path().join("data");
    seed_goal(&data_dir, "alpha", "2026-09-01T10:00:00Z");
    seed_goal(&data_dir, "bravo", "2026-09-03T10:00:00Z");
    seed_goal(&data_dir, "charlie", "2026-09-02T10:00:00Z");

    let daemon = start_daemon(dir.path());
    let client = DaemonClient::connect_to(daemon.socket_addr().path());

    assert!(
        goal_list(&client, Some(0)).is_empty(),
        "limit 0 must be an empty page, not an error"
    );

    let one = goal_list(&client, Some(1));
    assert_eq!(
        one.len(),
        1,
        "limit must truncate from the newest end, not pass the store through"
    );
    assert_eq!(one[0].id, "bravo", "the kept row is the most recent one");

    assert_eq!(
        goal_list(&client, None).len(),
        3,
        "a store smaller than the default page must come back in full"
    );
}

/// A data dir with no `todos.jsonl` / `goals.jsonl` at all (the CLI was
/// never run) answers an empty list, not an error — the board renders an
/// empty state on a fresh install.  Goes red if a missing file is
/// propagated as an error response instead of `[]`.
#[test]
fn todo_goal_list_empty_when_no_store() {
    let dir = TempDir::new().expect("temp dir");
    // `data` is never created: the handlers' `Store::new` makes it, and
    // the loaders treat an absent file as no records.
    let daemon = start_daemon(dir.path());
    let client = DaemonClient::connect_to(daemon.socket_addr().path());

    assert!(
        todo_list(&client, None).is_empty(),
        "an unseen data dir must answer an empty todo list, not fail"
    );
    assert!(
        goal_list(&client, None).is_empty(),
        "an unseen data dir must answer an empty goal list, not fail"
    );
}

/// `Goal.todo_ids` is one-directional and may dangle (a todo id with no
/// todo row).  The handler serializes the goal as stored — the link
/// survives the round trip untouched.  Goes red if the handler filters
/// dangling ids or drops the field.
#[test]
fn goal_list_serializes_todo_ids() {
    let dir = TempDir::new().expect("temp dir");
    let data_dir = dir.path().join("data");
    seed_todo(&data_dir, "alpha", "2026-09-01T10:00:00Z");
    let mut goal = Goal::new("Ship the board".to_string()).expect("goal");
    goal.link_todo("alpha".to_string());
    goal.link_todo("ghost-todo".to_string()); // dangling on purpose
    Store::new(&data_dir)
        .append_goal(&goal)
        .unwrap_or_else(|e| panic!("seed goal: {e}"));

    let daemon = start_daemon(dir.path());
    let client = DaemonClient::connect_to(daemon.socket_addr().path());

    let goals = goal_list(&client, None);
    assert_eq!(goals.len(), 1, "one goal was seeded");
    assert_eq!(
        goals[0].todo_ids,
        vec!["alpha".to_string(), "ghost-todo".to_string()],
        "todo_ids must round-trip as stored — dangling ids included"
    );
}
