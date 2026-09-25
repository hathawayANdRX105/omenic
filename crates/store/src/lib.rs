//! Task model, store, dependency graph, run flow.

pub mod goal;
pub mod graph;
pub mod jsonl;
pub mod store;
pub mod template;
pub mod todo;
use serde::{Deserialize, Serialize};

/// Whether a task is a milestone, feature, bug, plain task, chore, spike, or decision.
/// Unknown variants from old data deserialize as `Task` (backward compat).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Milestone,
    Feature,
    Bug,
    Task,
    Chore,
    Spike,
    Decision,
    /// Fallback for unknown/old variants (dep, template, etc).
    #[serde(other)]
    Unknown,
}

/// Default priority for tasks created before the priority field existed (P2).
fn default_priority() -> u8 {
    2
}

/// Progress state of a task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Open,
    InProgress,
    /// Last run attempt failed; retryable until the attempt budget is
    /// exhausted (#47). Distinct from InProgress so a crashed runner
    /// (orphan) can be told apart from a normal failure.
    Failed,
    Done,
}

/// A single work item in the task graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub kind: TaskKind,
    pub status: TaskStatus,
    /// Failed run attempts so far (#47). Zero for tasks that never ran or
    /// predate the field; gated against `runner::MAX_ATTEMPTS`.
    #[serde(default)]
    pub attempts: u32,
    #[serde(default = "default_priority")]
    pub priority: u8,
    pub parent: Option<String>,
    pub deps: Vec<String>,
    pub description: String,
    pub acceptance: String,
    pub created_at: String,
    pub updated_at: String,
}

/// ISO-8601-ish UTC timestamp (seconds precision). Shared by CLI and
/// template layers so task timestamps use one implementation.
pub fn now_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}
