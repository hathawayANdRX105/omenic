//! Tool-level tests for the todo/goal model tools.
//!
//! These exercise the real path end to end: a real `Store` over a real temp
//! jsonl file, the real state machine, and the real registration list. They
//! live in `tests/` like the rest of this crate's behaviour tests.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde_json::{Value, json};
use store::goal::GoalStatus;
use store::store::Store;
use store::todo::TodoStatus;
use store::tools::session_tools;
use tools::{Tool, ToolError};

fn tmp_tools() -> (tempfile::TempDir, Arc<Store>, Vec<Arc<dyn Tool>>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(Store::new(dir.path()));
    let tools = session_tools(Arc::clone(&store));
    (dir, store, tools)
}

fn call(tools: &[Arc<dyn Tool>], name: &str, args: Value) -> Result<String, ToolError> {
    let tool = tools
        .iter()
        .find(|t| t.name() == name)
        .unwrap_or_else(|| panic!("tool not registered: {name}"));
    tool.execute(&args, &AtomicBool::new(false))
}

// --- todos -------------------------------------------------------------------

#[test]
fn todo_add_creates_then_updates_note_only() {
    // Bug it catches: add on an existing title inserts a duplicate (the board
    // shows the item twice), or resets the status instead of only updating
    // the note (an in-progress todo silently reopens).
    let (_dir, store, tools) = tmp_tools();

    call(
        &tools,
        "todo_add",
        json!({"title": "write docs", "note": "draft"}),
    )
    .expect("first add");
    call(
        &tools,
        "todo_update",
        json!({"title": "write docs", "status": "in_progress"}),
    )
    .expect("move to in_progress");
    call(
        &tools,
        "todo_add",
        json!({"title": "write docs", "note": "second pass"}),
    )
    .expect("re-add updates the note");

    let todos = store.load_todos().expect("load");
    assert_eq!(todos.len(), 1, "re-add must not duplicate the todo");
    assert_eq!(todos[0].note.as_deref(), Some("second pass"));
    assert_eq!(
        todos[0].status,
        TodoStatus::InProgress,
        "re-add must keep the status"
    );
}

#[test]
fn todo_update_rejects_invalid_transition() {
    // Bug it catches: the state machine is bypassed (terminal states exchange
    // places), or the error text omits the from/to states so the model cannot
    // tell what went wrong and self-correct.
    let (_dir, store, tools) = tmp_tools();

    call(&tools, "todo_add", json!({"title": "ship"})).expect("add");
    call(
        &tools,
        "todo_update",
        json!({"title": "ship", "status": "done"}),
    )
    .expect("mark done");

    let err = call(
        &tools,
        "todo_update",
        json!({"title": "ship", "status": "cancelled"}),
    )
    .expect_err("done -> cancelled must be refused")
    .to_string();
    assert!(
        err.contains("done"),
        "error must name the from state: {err}"
    );
    assert!(
        err.contains("cancelled"),
        "error must name the to state: {err}"
    );

    let todos = store.load_todos().expect("load");
    assert_eq!(
        todos[0].status,
        TodoStatus::Done,
        "a refused transition must leave the todo done"
    );
}

#[test]
fn todo_update_missing_title_errors() {
    // Bug it catches: update degrades into an upsert, so a typo'd title
    // silently creates a fresh todo instead of reporting the miss.
    let (_dir, store, tools) = tmp_tools();

    let err = call(
        &tools,
        "todo_update",
        json!({"title": "ghost", "status": "done"}),
    )
    .expect_err("unknown title must error")
    .to_string();
    assert!(
        err.contains("not found"),
        "error should say not found: {err}"
    );
    assert!(
        store.load_todos().expect("load").is_empty(),
        "a failed update must not insert anything"
    );
}

#[test]
fn todo_add_empty_title_errors() {
    // Bug it catches: a blank title lands in the store, producing an unreadable
    // row the model cannot address again (its id is the title).
    let (_dir, store, tools) = tmp_tools();

    call(&tools, "todo_add", json!({"title": "  "})).expect_err("blank title must error");
    assert!(
        store.load_todos().expect("load").is_empty(),
        "blank title must not reach the store"
    );
}

// --- goals -------------------------------------------------------------------

#[test]
fn goal_add_links_idempotently() {
    // Bug it catches: link dedup fails, so re-linking a known todo id stores
    // it twice and every consumer renders the link list doubled.
    let (_dir, store, tools) = tmp_tools();

    call(
        &tools,
        "goal_add",
        json!({"title": "release", "todos": ["a", "b"]}),
    )
    .expect("create goal");
    let out = call(
        &tools,
        "goal_add",
        json!({"title": "release", "todos": ["b", "c"]}),
    )
    .expect("re-add links only the new ids");

    let goals = store.load_goals().expect("load");
    assert_eq!(goals.len(), 1);
    assert_eq!(goals[0].todo_ids, vec!["a", "b", "c"]);
    assert_eq!(goals[0].status, GoalStatus::Active);
    assert!(
        out.contains("linked: 3"),
        "output must report the merged link count: {out}"
    );
}

#[test]
fn goal_link_unlink_roundtrip() {
    // Bug it catches: unlink is not idempotent — a second unlink of an absent
    // id errors (or panics) instead of no-opping, so a retrying model gets
    // stuck on a link that is already gone.
    let (_dir, store, tools) = tmp_tools();

    call(&tools, "goal_add", json!({"title": "g", "todos": ["a"]})).expect("create goal");
    let out = call(
        &tools,
        "goal_link",
        json!({"title": "g", "todo": "a", "unlink": true}),
    )
    .expect("unlink");
    assert!(out.contains("linked: 0"), "unlink must drop the id: {out}");

    let out = call(
        &tools,
        "goal_link",
        json!({"title": "g", "todo": "a", "unlink": true}),
    )
    .expect("second unlink must be a no-op");
    assert!(out.contains("linked: 0"), "still empty: {out}");

    let goals = store.load_goals().expect("load");
    assert!(goals[0].todo_ids.is_empty());
}

#[test]
fn goal_link_missing_goal_errors() {
    // Bug it catches: linking against an unknown goal silently creates it,
    // masking a caller bug (a typo'd goal title becomes a real goal with one
    // link) instead of failing loudly.
    let (_dir, store, tools) = tmp_tools();

    let err = call(&tools, "goal_link", json!({"title": "ghost", "todo": "a"}))
        .expect_err("unknown goal must error")
        .to_string();
    assert!(
        err.contains("not found"),
        "error should say not found: {err}"
    );
    assert!(
        store.load_goals().expect("load").is_empty(),
        "a failed link must not create the goal"
    );
}

// --- registration ------------------------------------------------------------

#[test]
fn tools_are_registered_in_order() {
    // Bug it catches: registration order drifts (a tool is dropped or renamed),
    // so the daemon silently assembles a session missing a tool the model was
    // told it has.
    let (_dir, _store, tools) = tmp_tools();

    let names: Vec<String> = tools.iter().map(|t| tools::def(t.as_ref()).name).collect();
    assert_eq!(
        names,
        vec![
            "todo_add",
            "todo_update",
            "todo_list",
            "goal_add",
            "goal_link"
        ]
    );
}
