//! Behaviour tests for the todo/goal tracking model and its jsonl store.
//!
//! These cover the state machine, the link invariants, and the storage
//! round-trip. They are integration tests living in `tests/`, so they need
//! no test-cfg attribute — the observable contract is what a caller in
//! another crate sees.

use store::Task;
use store::goal::{Goal, GoalError, GoalStatus};
use store::store::Store;
use store::todo::{Todo, TodoError, TodoStatus};

fn tmp_store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::new(dir.path());
    (dir, store)
}

// --- state machine ---------------------------------------------------------

#[test]
fn invalid_transition_done_to_cancelled_is_rejected() {
    // Bug it catches: the state machine lets two terminal states exchange,
    // so a finished todo could be retroactively marked "cancelled, never
    // done" and the panel can no longer tell success from abandonment.
    let mut todo = Todo::new("ship v1".into()).unwrap();
    todo.transition(TodoStatus::InProgress).unwrap();
    todo.transition(TodoStatus::Done).unwrap();

    let err = todo.transition(TodoStatus::Cancelled).unwrap_err();
    assert!(matches!(err, TodoError::InvalidTransition { .. }));
    assert_eq!(todo.status, TodoStatus::Done, "done todo must stay done");
}

#[test]
fn cancelled_is_terminal() {
    // Bug it catches: cancelled is not closed, so a cancelled todo can be
    // flipped back open (or to done) and a dropped item silently reappears.
    let mut todo = Todo::new("dead end".into()).unwrap();
    todo.transition(TodoStatus::Cancelled).unwrap();

    for to in [
        TodoStatus::Open,
        TodoStatus::InProgress,
        TodoStatus::Done,
        TodoStatus::Cancelled,
    ] {
        // transition() takes `to` by value; clone so the message below still borrows it.
        let err = todo.transition(to.clone()).unwrap_err();
        assert!(
            matches!(err, TodoError::InvalidTransition { .. }),
            "cancelled -> {to:?} must be rejected"
        );
    }
    assert_eq!(todo.status, TodoStatus::Cancelled);
}

#[test]
fn done_can_be_reopened() {
    // Bug it catches: the machine is over-strict and rejects the documented
    // reopen path, so a wrongly-closed todo can never be worked again.
    let mut todo = Todo::new("reopened".into()).unwrap();
    todo.transition(TodoStatus::InProgress).unwrap();
    todo.transition(TodoStatus::Done).unwrap();
    todo.transition(TodoStatus::Open)
        .expect("done -> open is a reopen");
    assert_eq!(todo.status, TodoStatus::Open);

    // And the reopened item can still proceed normally afterwards.
    todo.transition(TodoStatus::InProgress).unwrap();
    todo.transition(TodoStatus::Done).unwrap();
    assert_eq!(todo.status, TodoStatus::Done);
}

#[test]
fn invalid_transition_error_names_both_states() {
    // Bug it catches: the error drops the `from`/`to` context, so a caller
    // (or a user reading the message) cannot tell which side was wrong.
    let mut todo = Todo::new("audit".into()).unwrap();
    todo.transition(TodoStatus::Done).unwrap();
    match todo.transition(TodoStatus::Cancelled).unwrap_err() {
        TodoError::InvalidTransition { from, to } => {
            assert_eq!(from, TodoStatus::Done);
            assert_eq!(to, TodoStatus::Cancelled);
        }
        other => panic!("expected InvalidTransition, got {other:?}"),
    }
}

// --- model construction ----------------------------------------------------

#[test]
fn empty_title_rejected() {
    // Bug it catches: validation is missing, so a blank title lands in the
    // store and the panel renders an empty row that cannot be addressed
    // (the title is the id — there would be nothing to key it by).
    assert!(matches!(
        Todo::new(String::new()).unwrap_err(),
        TodoError::EmptyTitle
    ));
    assert!(matches!(
        Todo::new("   ".into()).unwrap_err(),
        TodoError::EmptyTitle
    ));
    assert!(matches!(
        Goal::new(String::new()).unwrap_err(),
        GoalError::EmptyTitle
    ));

    let todo = Todo::new("  trimmed  ".into()).unwrap();
    assert_eq!(todo.title, "trimmed");
    assert_eq!(todo.id, "trimmed", "id follows the trimmed title");
    assert_eq!(todo.status, TodoStatus::Open);
    assert_eq!(todo.note, None);

    let goal = Goal::new("release".into()).unwrap();
    assert_eq!(goal.status, GoalStatus::Active);
    assert!(goal.todo_ids.is_empty());
}

// --- goal <-> todo links ---------------------------------------------------

#[test]
fn link_todo_is_idempotent() {
    // Bug it catches: the dedup guard is missing, so re-linking an already
    // linked todo inserts a duplicate and the panel renders it twice.
    let mut goal = Goal::new("release".into()).unwrap();
    goal.link_todo("write release notes".into());
    goal.link_todo("write release notes".into());
    goal.link_todo("tag the repo".into());
    assert_eq!(goal.todo_ids, vec!["write release notes", "tag the repo"]);
}

#[test]
fn unlink_todo_is_idempotent() {
    // Bug it catches: unlinking an unknown id panics or wipes the list
    // instead of being a no-op.
    let mut goal = Goal::new("release".into()).unwrap();
    goal.link_todo("a".into());
    goal.link_todo("b".into());

    goal.unlink_todo("not-linked");
    assert_eq!(goal.todo_ids, vec!["a", "b"]);

    goal.unlink_todo("a");
    assert_eq!(goal.todo_ids, vec!["b"]);

    // Unlinking an already-unlinked id is still a no-op.
    goal.unlink_todo("a");
    assert_eq!(goal.todo_ids, vec!["b"]);
}

// --- storage ---------------------------------------------------------------

#[test]
fn store_roundtrip_preserves_order() {
    // Bug it catches: serialization drops a field, or load does not sort
    // deterministically, so two loads of the same file yield different order.
    let (_dir, store) = tmp_store();

    let mut todos = vec![
        Todo::new("mid".into()).unwrap(),
        Todo::new("zap".into()).unwrap(),
        Todo::new("apple".into()).unwrap(),
    ];
    todos[0].transition(TodoStatus::InProgress).unwrap();
    todos[0].note = Some("working".into());
    for todo in &todos {
        store.append_todo(todo).unwrap();
    }

    let loaded = store.load_todos().unwrap();
    assert_eq!(
        loaded.iter().map(|t| t.id.clone()).collect::<Vec<_>>(),
        vec!["apple", "mid", "zap"],
        "load must return id-sorted records regardless of append order"
    );
    let mid = loaded.iter().find(|t| t.id == "mid").unwrap();
    assert_eq!(mid.status, TodoStatus::InProgress);
    assert_eq!(mid.note.as_deref(), Some("working"));
    assert_eq!(mid.title, "mid");
    // Every record keeps its own pair of timestamps.
    assert!(
        !loaded
            .iter()
            .any(|t| t.created_at.is_empty() || t.updated_at.is_empty())
    );
}

#[test]
fn append_todo_latest_wins_for_same_id() {
    // Bug it catches: dedup by id is broken (earliest wins, or duplicates
    // survive), so updating a todo leaves the stale record on top and the
    // panel shows an open item that was marked done.
    let (_dir, store) = tmp_store();

    let mut todo = Todo::new("single".into()).unwrap();
    store.append_todo(&todo).unwrap();
    todo.transition(TodoStatus::InProgress).unwrap();
    store.append_todo(&todo).unwrap();
    todo.transition(TodoStatus::Done).unwrap();
    store.append_todo(&todo).unwrap();

    let loaded = store.load_todos().unwrap();
    assert_eq!(loaded.len(), 1, "same id must collapse to one record");
    assert_eq!(loaded[0].id, "single");
    assert_eq!(loaded[0].status, TodoStatus::Done);
}

#[test]
fn store_keeps_tasks_todos_and_goals_separate() {
    // Bug it catches: the three record kinds share a file or a keying bug
    // mixes them, so appending a goal poisons the todo list.
    let (_dir, store) = tmp_store();

    store
        .append(&Task {
            id: "task-1".into(),
            title: "task-1".into(),
            kind: store::TaskKind::Task,
            status: store::TaskStatus::Open,
            attempts: 0,
            priority: 2,
            parent: None,
            deps: vec![],
            description: String::new(),
            acceptance: String::new(),
            created_at: store::now_iso(),
            updated_at: store::now_iso(),
        })
        .unwrap();
    store
        .append_todo(&Todo::new("todo-1".into()).unwrap())
        .unwrap();
    store
        .append_goal(&Goal::new("goal-1".into()).unwrap())
        .unwrap();

    assert_eq!(store.load_all().unwrap().len(), 1);
    assert_eq!(store.load_todos().unwrap().len(), 1);
    assert_eq!(store.load_goals().unwrap().len(), 1);
}

#[test]
fn load_on_missing_files_returns_empty() {
    // Bug it catches: a fresh data dir (no files yet) errors instead of
    // returning an empty list, so a first-run panel cannot boot.
    let (_dir, store) = tmp_store();
    assert!(store.load_todos().unwrap().is_empty());
    assert!(store.load_goals().unwrap().is_empty());
    assert!(store.load_all().unwrap().is_empty());
}

#[test]
fn goal_roundtrip_keeps_todo_links() {
    // Bug it catches: the todo_ids vector is not serialized, so a goal loses
    // every link on reload and the panel shows a goal with no children.
    let (_dir, store) = tmp_store();

    let mut goal = Goal::new("release".into()).unwrap();
    goal.link_todo("write release notes".into());
    goal.link_todo("tag the repo".into());
    store.append_goal(&goal).unwrap();

    let loaded = store.load_goals().unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(
        loaded[0].todo_ids,
        vec!["write release notes", "tag the repo"],
        "link order must survive the round-trip"
    );

    // An unlink + re-append updates the same goal id in place.
    let mut updated = loaded.into_iter().next().unwrap();
    updated.unlink_todo("tag the repo");
    store.append_goal(&updated).unwrap();
    let reloaded = store.load_goals().unwrap();
    assert_eq!(reloaded.len(), 1);
    assert_eq!(reloaded[0].todo_ids, vec!["write release notes"]);
}
