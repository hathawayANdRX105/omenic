//! Tracking-model todo: a human-facing checklist item.
//!
//! Distinct from [`crate::Task`], which is the *execution* model (units the
//! runner picks up and runs). A todo is what a person reads to see where a
//! line of work stands. State machine:
//!
//! ```text
//! open ──▶ in_progress ──▶ done
//!  │           │             │
//!  └───────────┴────────────▶┤ (reopen: done → open allowed)
//!                            │
//!                       cancelled (terminal, mutually exclusive with done)
//! ```

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::now_iso;

/// Lifecycle state of a [`Todo`]. Serialized `snake_case` to match
/// [`crate::TaskStatus`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Open,
    InProgress,
    Done,
    /// Terminal: a cancelled todo is closed without being finished, so it can
    /// never reopen and can never be flipped to `Done`.
    Cancelled,
}

/// A single human-facing checklist item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Todo {
    pub id: String,
    pub title: String,
    pub status: TodoStatus,
    pub created_at: String,
    pub updated_at: String,
    pub note: Option<String>,
}

/// Errors from todo operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TodoError {
    /// `title` was empty (or whitespace only).
    EmptyTitle,
    /// The requested status change is not allowed from the current state.
    InvalidTransition { from: TodoStatus, to: TodoStatus },
}

impl fmt::Display for TodoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TodoError::EmptyTitle => write!(f, "todo title must be non-empty"),
            TodoError::InvalidTransition { from, to } => {
                write!(f, "invalid todo transition: {from:?} -> {to:?}")
            }
        }
    }
}

impl std::error::Error for TodoError {}

impl Todo {
    /// Create a new open todo. The id is the trimmed title (same convention as
    /// [`crate::Task`]: appending a todo with a known id updates it, so a
    /// status change is just another append).
    ///
    /// Returns [`TodoError::EmptyTitle`] for a blank title.
    pub fn new(title: String) -> Result<Self, TodoError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(TodoError::EmptyTitle);
        }
        let now = now_iso();
        Ok(Todo {
            id: title.to_string(),
            title: title.to_string(),
            status: TodoStatus::Open,
            created_at: now.clone(),
            updated_at: now,
            note: None,
        })
    }

    /// Move this todo to `to`, if the state machine allows it.
    ///
    /// Allowed: anything except out of `Cancelled`, and except `Done` →
    /// `Cancelled` (a finished item was not cancelled; the two terminals are
    /// mutually exclusive). Reopening (`Done` → `Open`) is allowed.
    pub fn transition(&mut self, to: TodoStatus) -> Result<(), TodoError> {
        match (&self.status, &to) {
            (TodoStatus::Cancelled, _) | (TodoStatus::Done, TodoStatus::Cancelled) => {
                Err(TodoError::InvalidTransition {
                    from: self.status.clone(),
                    to,
                })
            }
            _ => {
                self.status = to;
                self.updated_at = now_iso();
                Ok(())
            }
        }
    }
}
