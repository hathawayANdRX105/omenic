//! Tool-level tests for the todo/goal model tools.
//!
//! These exercise the real path end to end: a real `Store` over a real temp
//! jsonl file, the real state machine, and the real registration list.

use std::sync::Arc;

use serde_json::{Value, json};
use store::Store;
use tools::task::session_tools;
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
    let (_dir, store, tools) = tmp_tools();
    let result = call(
        &tools,
        "todo_add",
        json!({ "title": "first", "note": "initial" }),
    )
    .expect("todo_add ok");
    assert!(result.contains("first"));
    assert!(result.contains("initial"));

    // Re-adding the same title should update only the note, not create a duplicate
    let result2 = call(
        &tools,
        "todo_add",
        json!({ "title": "first", "note": "updated" }),
    )
    .expect("todo_add ok");
    assert!(result2.contains("updated"));
    assert!(!result2.contains("initial"));

    // Verify store state: only one todo with that title
    let todos = store.load_todos().expect("list");
    let matches: Vec<_> = todos.iter().filter(|t| t.title == "first").collect();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].note.as_deref(), Some("updated"));
}

#[test]
fn todo_update_rejects_invalid_transition() {
    let (_dir, _store, tools) = tmp_tools();
    call(&tools, "todo_add", json!({ "title": "task" })).unwrap();
    // Move to a terminal state.
    call(
        &tools,
        "todo_update",
        json!({ "title": "task", "status": "done" }),
    )
    .unwrap();
    // Terminals are mutually exclusive: a done todo cannot become cancelled.
    let err = call(
        &tools,
        "todo_update",
        json!({ "title": "task", "status": "cancelled" }),
    )
    .expect_err("done -> cancelled rejected");
    assert!(err.to_string().contains("invalid todo transition"));
}

#[test]
fn todo_update_missing_title_errors() {
    let (_dir, _store, tools) = tmp_tools();
    let err = call(
        &tools,
        "todo_update",
        json!({ "title": "nonexistent", "status": "done" }),
    )
    .expect_err("missing title errors");
    assert!(err.to_string().contains("not found"));
}

#[test]
fn todo_add_empty_title_errors() {
    let (_dir, _store, tools) = tmp_tools();
    let err = call(&tools, "todo_add", json!({ "title": "" })).expect_err("empty title rejected");
    assert!(err.to_string().contains("empty"));
}

// --- goals -------------------------------------------------------------------

#[test]
fn goal_add_links_idempotently() {
    let (_dir, store, tools) = tmp_tools();
    // Add a todo first
    call(&tools, "todo_add", json!({ "title": "todo1" })).unwrap();
    call(&tools, "todo_add", json!({ "title": "todo2" })).unwrap();

    // Create goal linking both
    let res = call(
        &tools,
        "goal_add",
        json!({
            "title": "Goal",
            "todos": ["todo1", "todo2"]
        }),
    )
    .expect("goal_add ok");
    assert!(res.contains("Goal"));

    // Second add with same title should be idempotent
    let res2 = call(
        &tools,
        "goal_add",
        json!({
            "title": "Goal",
            "todos": ["todo1", "todo2"]
        }),
    )
    .expect("goal_add idempotent");
    assert!(res2.contains("Goal"));

    // Verify store state: one goal, two links
    let goals = store.load_goals().expect("load goals");
    assert_eq!(goals.len(), 1);
    assert_eq!(goals[0].title, "Goal");
    assert_eq!(goals[0].todo_ids.len(), 2);
}

#[test]
fn goal_link_unlink_roundtrip() {
    let (_dir, store, tools) = tmp_tools();
    call(&tools, "todo_add", json!({ "title": "t1" })).unwrap();
    call(&tools, "todo_add", json!({ "title": "t2" })).unwrap();
    call(&tools, "goal_add", json!({ "title": "G" })).unwrap();

    // Link t1
    let res = call(&tools, "goal_link", json!({ "title": "G", "todo": "t1" })).unwrap();
    assert!(res.contains("linked"));

    // Link t2
    let res = call(&tools, "goal_link", json!({ "title": "G", "todo": "t2" })).unwrap();
    assert!(res.contains("linked"));

    // Unlink t1
    let res = call(
        &tools,
        "goal_link",
        json!({ "title": "G", "todo": "t1", "unlink": true }),
    )
    .unwrap();
    assert!(res.contains("linked"));

    let goals = store.load_goals().expect("load goals");
    let g = goals.iter().find(|g| g.title == "G").expect("goal G");
    assert_eq!(g.todo_ids.len(), 1);
    assert_eq!(g.todo_ids[0], "t2");
}

#[test]
fn goal_link_missing_goal_errors() {
    let (_dir, _store, tools) = tmp_tools();
    let err = call(
        &tools,
        "goal_link",
        json!({ "title": "nonexistent", "todo": "t" }),
    )
    .expect_err("missing goal rejected");
    assert!(err.to_string().contains("not found"));
}

// --- registration ------------------------------------------------------------

#[test]
fn tools_are_registered_in_order() {
    let (_dir, _store, tools) = tmp_tools();
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
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
