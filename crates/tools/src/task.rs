//! Model-facing tools for the todo/goal tracking model.
//!
//! Until now the todo/goal model existed only as storage plus CLI-less model
//! code: nothing in an orbit engine could write it. These five tools close
//! that gap — the model maintains its own checklist (`todo_add` /
//! `todo_update` / `todo_list`) and groups items under coarse outcomes
//! (`goal_add` / `goal_link`).
//!
//! # Which `Tool` trait
//!
//! These implement the **agent-domain** `tools::Tool` (name / description /
//! parameters / execute), not the harness `Tool`. The engine dispatches that
//! trait and MCP tools arrive in the same shape, so the daemon can hand these
//! to an engine directly with no adapter. The precedent — and the C6 rationale
//! for the two traits staying separate — is
//! `crates/harness/tools/src/jobs_terminal.rs`.
//!
//! # Sharing
//!
//! One `Arc<Store>` per session, shared by all five tools: `session_tools` is
//! called once and the handles are cloned into every engine (re)spawn, so a
//! todo written in one turn is visible in the next. The store is append-only
//! jsonl behind fcntl locks, so every clone observes every write.
//!
//! # Concurrency boundary
//!
//! Every tool is a **load-modify-append** sequence under two separate store
//! locks — the store has no cross-operation transaction. Inside one engine
//! tool calls are serialized, so this is safe. Forked subagents sharing the
//! same `Arc<Store>` may race on the same id: latest-wins append semantics
//! mean one of the two concurrent updates can be lost. That is acceptable for
//! a human-facing checklist and is deliberately not solved in this slice.
//!
//! # The abort signal
//!
//! Checked once at the top of `execute`; the file operations below are short
//! enough not to poll it again.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{Tool, ToolError, arg_str};
use serde_json::{Value, json};

use store::goal::{Goal, GoalError, GoalStatus};
use store::store::{Store, StoreError};
use store::todo::{Todo, TodoError, TodoStatus};

// -----------------------------------------------------------------------------
// Argument helpers
// -----------------------------------------------------------------------------

/// Optional string argument, trimmed at the call site when it is an id.
fn opt_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_string)
}

/// String-array argument; absent means empty. Non-string elements are an
/// error rather than a silent skip — a dropped link is a bug the model cannot
/// see from the output.
fn str_array(args: &Value, key: &str) -> Result<Vec<String>, ToolError> {
    let Some(items) = args.get(key).and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    items
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::trim)
                .map(str::to_string)
                .ok_or_else(|| ToolError::Message(format!("{key} must be an array of strings")))
        })
        .collect()
}

/// Parse a todo status word. The four legal values are listed in the error so
/// the model can self-correct on a typo like `in-progress`.
fn parse_status(s: &str) -> Result<TodoStatus, ToolError> {
    match s {
        "open" => Ok(TodoStatus::Open),
        "in_progress" => Ok(TodoStatus::InProgress),
        "done" => Ok(TodoStatus::Done),
        "cancelled" => Ok(TodoStatus::Cancelled),
        other => Err(ToolError::Message(format!(
            "invalid todo status: {other} (expected one of: open, in_progress, done, cancelled)"
        ))),
    }
}

// -----------------------------------------------------------------------------
// Error mapping
// -----------------------------------------------------------------------------

/// Store failures keep the underlying error text: an IO or serialization
/// problem is the model's problem too, and swallowing it would look like an
/// empty board.
fn store_err(e: StoreError) -> ToolError {
    ToolError::Message(format!("store error: {e}"))
}

fn todo_err(e: TodoError) -> ToolError {
    ToolError::Message(e.to_string())
}

fn goal_err(e: GoalError) -> ToolError {
    ToolError::Message(e.to_string())
}

/// A refused transition names both states in the stored (snake_case)
/// vocabulary, so the error doubles as a mini state-machine reference.
fn transition_err(e: TodoError) -> ToolError {
    match e {
        TodoError::InvalidTransition { from, to } => ToolError::Message(format!(
            "invalid todo transition: {} -> {}; todo stays {}",
            status_word(&from),
            status_word(&to),
            status_word(&from)
        )),
        other => ToolError::Message(other.to_string()),
    }
}

// -----------------------------------------------------------------------------
// Formatting
// -----------------------------------------------------------------------------

fn status_word(s: &TodoStatus) -> &'static str {
    match s {
        TodoStatus::Open => "open",
        TodoStatus::InProgress => "in_progress",
        TodoStatus::Done => "done",
        TodoStatus::Cancelled => "cancelled",
    }
}

fn goal_status_word(s: &GoalStatus) -> &'static str {
    match s {
        GoalStatus::Active => "active",
        GoalStatus::Achieved => "achieved",
        GoalStatus::Abandoned => "abandoned",
    }
}

/// One-line-per-fact result for a write, with board-wide counts so the model
/// sees the effect of its call without a follow-up `todo_list`.
fn todo_report(todo: &Todo, todos: &[Todo]) -> String {
    let mut out = format!("todo: {} [{}]", todo.title, status_word(&todo.status));
    if let Some(note) = &todo.note {
        out.push_str(&format!("\nnote: {note}"));
    }
    let (open, wip, done, cancelled) = todos.iter().fold((0, 0, 0, 0), |mut c, t| {
        match t.status {
            TodoStatus::Open => c.0 += 1,
            TodoStatus::InProgress => c.1 += 1,
            TodoStatus::Done => c.2 += 1,
            TodoStatus::Cancelled => c.3 += 1,
        }
        c
    });
    out.push_str(&format!(
        "\ntodos: open {open} · in_progress {wip} · done {done} · cancelled {cancelled}"
    ));
    out
}

fn goal_report(goal: &Goal) -> String {
    let mut out = format!(
        "goal: {} [{}]\nlinked: {}",
        goal.title,
        goal_status_word(&goal.status),
        goal.todo_ids.len()
    );
    for id in &goal.todo_ids {
        out.push_str(&format!("\n  - {id}"));
    }
    out
}

/// Abort once at the door; the store calls below are fast file ops.
fn check_abort(signal: &AtomicBool) -> Result<(), ToolError> {
    if signal.load(Ordering::Relaxed) {
        return Err(ToolError::Message("aborted".into()));
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// todo_add
// -----------------------------------------------------------------------------

pub struct TodoAdd {
    store: Arc<Store>,
}

impl TodoAdd {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }
}

impl Tool for TodoAdd {
    fn name(&self) -> &str {
        "todo_add"
    }

    fn description(&self) -> String {
        "Add a todo to the checklist. The title doubles as its id: adding a title that already exists updates only its note and keeps its status.".into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Todo title; doubles as its id. A title that already exists updates that todo's note."
                },
                "note": {
                    "type": "string",
                    "description": "Optional free-text note (progress, blockers)."
                }
            },
            "required": ["title"]
        })
    }

    fn execute(&self, args: &Value, signal: &AtomicBool) -> Result<String, ToolError> {
        check_abort(signal)?;
        let title = arg_str(args, "title")?.trim();
        let note = opt_str(args, "note");

        let mut todos = self.store.load_todos().map_err(store_err)?;
        let todo = match todos.iter_mut().find(|t| t.id == title) {
            // Same title = same id, and latest-wins append is the update path:
            // only the note moves. A re-add must not resurrect a closed todo.
            Some(existing) => {
                existing.note = note;
                existing.updated_at = store::now_iso();
                existing.clone()
            }
            // A blank title matches no id and lands here, surfacing
            // Todo::new's own EmptyTitle text.
            None => {
                let mut todo = Todo::new(title.to_string()).map_err(todo_err)?;
                todo.note = note;
                todo
            }
        };
        self.store.append_todo(&todo).map_err(store_err)?;
        let todos = self.store.load_todos().map_err(store_err)?;
        Ok(todo_report(&todo, &todos))
    }
}

// -----------------------------------------------------------------------------
// todo_update
// -----------------------------------------------------------------------------

pub struct TodoUpdate {
    store: Arc<Store>,
}

impl TodoUpdate {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }
}

impl Tool for TodoUpdate {
    fn name(&self) -> &str {
        "todo_update"
    }

    fn description(&self) -> String {
        "Change a todo's status (open|in_progress|done|cancelled) and optionally its note. Terminal states are final: a done todo cannot become cancelled, and a cancelled one cannot move at all.".into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Todo title (its id); must already exist."
                },
                "status": {
                    "type": "string",
                    "enum": ["open", "in_progress", "done", "cancelled"],
                    "description": "New status. done and cancelled are terminal and mutually exclusive."
                },
                "note": {
                    "type": "string",
                    "description": "Optional new note; omit to keep the current one."
                }
            },
            "required": ["title", "status"]
        })
    }

    fn execute(&self, args: &Value, signal: &AtomicBool) -> Result<String, ToolError> {
        check_abort(signal)?;
        let title = arg_str(args, "title")?.trim();
        let to = parse_status(arg_str(args, "status")?)?;
        let note = opt_str(args, "note");

        let mut todos = self.store.load_todos().map_err(store_err)?;
        let todo = todos
            .iter_mut()
            .find(|t| t.id == title)
            .ok_or_else(|| ToolError::Message(format!("todo not found: {title}")))?;
        todo.transition(to).map_err(transition_err)?;
        if let Some(note) = note {
            todo.note = Some(note);
        }
        let todo = todo.clone();
        self.store.append_todo(&todo).map_err(store_err)?;
        let todos = self.store.load_todos().map_err(store_err)?;
        Ok(todo_report(&todo, &todos))
    }
}

// -----------------------------------------------------------------------------
// todo_list
// -----------------------------------------------------------------------------

pub struct TodoList {
    store: Arc<Store>,
}

impl TodoList {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }
}

impl Tool for TodoList {
    fn name(&self) -> &str {
        "todo_list"
    }

    fn description(&self) -> String {
        "List all todos with their status and note, sorted by title.".into()
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    fn execute(&self, _args: &Value, signal: &AtomicBool) -> Result<String, ToolError> {
        check_abort(signal)?;
        let todos = self.store.load_todos().map_err(store_err)?;
        if todos.is_empty() {
            return Ok("no todos".into());
        }
        let mut out = String::new();
        for todo in &todos {
            out.push_str(&format!("- [{}] {}", status_word(&todo.status), todo.title));
            if let Some(note) = &todo.note
                && !note.is_empty()
            {
                out.push_str(&format!(" — {note}"));
            }
            out.push('\n');
        }
        Ok(out.trim_end().to_string())
    }
}

// -----------------------------------------------------------------------------
// goal_add
// -----------------------------------------------------------------------------

pub struct GoalAdd {
    store: Arc<Store>,
}

impl GoalAdd {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }
}

impl Tool for GoalAdd {
    fn name(&self) -> &str {
        "goal_add"
    }

    fn description(&self) -> String {
        "Add a goal (a coarse outcome) and link todos to it. Re-adding an existing goal links only the todo ids it does not have yet.".into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Goal title; doubles as its id. An existing title updates that goal's links."
                },
                "todos": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Todo titles to link. Already-linked ids are skipped."
                }
            },
            "required": ["title"]
        })
    }

    fn execute(&self, args: &Value, signal: &AtomicBool) -> Result<String, ToolError> {
        check_abort(signal)?;
        let title = arg_str(args, "title")?.trim();
        let links = str_array(args, "todos")?;

        let mut goals = self.store.load_goals().map_err(store_err)?;
        let goal = match goals.iter_mut().find(|g| g.id == title) {
            // Idempotent by construction: link_todo skips known ids.
            Some(existing) => {
                for id in links {
                    existing.link_todo(id);
                }
                existing.clone()
            }
            // A blank title matches no id and surfaces Goal::new's EmptyTitle.
            None => {
                let mut goal = Goal::new(title.to_string()).map_err(goal_err)?;
                for id in links {
                    goal.link_todo(id);
                }
                goal
            }
        };
        self.store.append_goal(&goal).map_err(store_err)?;
        Ok(goal_report(&goal))
    }
}

// -----------------------------------------------------------------------------
// goal_link
// -----------------------------------------------------------------------------

pub struct GoalLink {
    store: Arc<Store>,
}

impl GoalLink {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }
}

impl Tool for GoalLink {
    fn name(&self) -> &str {
        "goal_link"
    }

    fn description(&self) -> String {
        "Link a todo to an existing goal, or unlink it with unlink=true. Both directions are idempotent.".into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Goal title (its id); must already exist."
                },
                "todo": {
                    "type": "string",
                    "description": "Todo title to link or unlink."
                },
                "unlink": {
                    "type": "boolean",
                    "description": "Set true to remove the link instead of adding it. Default false."
                }
            },
            "required": ["title", "todo"]
        })
    }

    fn execute(&self, args: &Value, signal: &AtomicBool) -> Result<String, ToolError> {
        check_abort(signal)?;
        let title = arg_str(args, "title")?.trim();
        let todo = arg_str(args, "todo")?.trim();
        let unlink = args.get("unlink").and_then(Value::as_bool).unwrap_or(false);

        let mut goals = self.store.load_goals().map_err(store_err)?;
        let goal = goals
            .iter_mut()
            .find(|g| g.id == title)
            .ok_or_else(|| ToolError::Message(format!("goal not found: {title}")))?;
        if unlink {
            goal.unlink_todo(todo);
        } else {
            goal.link_todo(todo.to_string());
        }
        let goal = goal.clone();
        self.store.append_goal(&goal).map_err(store_err)?;
        Ok(goal_report(&goal))
    }
}

// -----------------------------------------------------------------------------
// Registration
// -----------------------------------------------------------------------------

/// Every todo/goal tool, backed by the given shared store.
///
/// The daemon calls this once per session and shares the resulting handles
/// across engine respawns (see the module docs). Returned as
/// `Arc<dyn Tool>` so a clone is a pointer copy and every engine observes the
/// same store. Registration order is part of the contract: the daemon and
/// its tests assert on these names.
pub fn session_tools(store: Arc<Store>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(TodoAdd::new(Arc::clone(&store))),
        Arc::new(TodoUpdate::new(Arc::clone(&store))),
        Arc::new(TodoList::new(Arc::clone(&store))),
        Arc::new(GoalAdd::new(Arc::clone(&store))),
        Arc::new(GoalLink::new(store)),
    ]
}
