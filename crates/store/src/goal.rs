//! Tracking-model goal: a coarse outcome that groups related todos.
//!
//! A goal holds a set of [`crate::todo::Todo`] ids (`todo_ids`). The link is
//! one-directional on purpose — a todo does not know its goal — so unlinking
//! or deleting a todo never leaves a back-reference dangling in the other
//! file. Whether a goal is achieved is judged by a human (or a later rule);
//! this model does not auto-complete goals from todo statuses.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::now_iso;

/// Lifecycle state of a [`Goal`]. Serialized `snake_case`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Active,
    Achieved,
    Abandoned,
}

/// A coarse outcome grouping several todos.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Goal {
    pub id: String,
    pub title: String,
    pub status: GoalStatus,
    /// Ids of the todos tracked by this goal. Order is insertion order.
    pub todo_ids: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Errors from goal operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalError {
    /// `title` was empty (or whitespace only).
    EmptyTitle,
}

impl fmt::Display for GoalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GoalError::EmptyTitle => write!(f, "goal title must be non-empty"),
        }
    }
}

impl std::error::Error for GoalError {}

impl Goal {
    /// Create a new active goal with no linked todos. The id is the trimmed
    /// title, matching [`crate::Todo`].
    pub fn new(title: String) -> Result<Self, GoalError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(GoalError::EmptyTitle);
        }
        let now = now_iso();
        Ok(Goal {
            id: title.to_string(),
            title: title.to_string(),
            status: GoalStatus::Active,
            todo_ids: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
        })
    }

    /// Link `todo_id` to this goal. Idempotent: an already-linked id is not
    /// added a second time.
    pub fn link_todo(&mut self, todo_id: String) {
        if !self.todo_ids.contains(&todo_id) {
            self.todo_ids.push(todo_id);
            self.updated_at = now_iso();
        }
    }

    /// Unlink `todo_id` from this goal. Idempotent: a unknown id is a no-op.
    pub fn unlink_todo(&mut self, todo_id: &str) {
        if self.todo_ids.iter().any(|t| t == todo_id) {
            self.todo_ids.retain(|t| t != todo_id);
            self.updated_at = now_iso();
        }
    }
}
