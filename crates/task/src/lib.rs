//! Task model, store, dependency graph, run flow.

pub mod graph;
pub mod runner;
pub mod store;
pub mod template;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construct_and_serialize() {
        let t = Task {
            id: "my-task".into(),
            title: "My Task".into(),
            kind: TaskKind::Task,
            status: TaskStatus::InProgress,
            attempts: 0,
            priority: 1,
            parent: None,
            deps: vec![],
            description: "Do the thing".into(),
            acceptance: String::new(),
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-02T00:00:00Z".into(),
        };
        let json = serde_json::to_string(&t).expect("serialize");
        assert!(json.contains(r#""kind":"task""#));
        assert!(json.contains(r#""status":"in_progress""#));
        assert!(json.contains(r#""priority":1"#));
    }

    #[test]
    fn roundtrip() {
        let t = Task {
            id: "roundtrip".into(),
            title: "Round Trip".into(),
            kind: TaskKind::Feature,
            status: TaskStatus::Done,
            attempts: 0,
            priority: 0,
            parent: Some("parent-task".into()),
            deps: vec!["dep-a".into(), "dep-b".into()],
            description: "Roundtrip test".into(),
            acceptance: "Must pass".into(),
            created_at: "2026-06-15T12:00:00Z".into(),
            updated_at: "2026-06-15T12:30:00Z".into(),
        };
        let json = serde_json::to_string(&t).expect("serialize");
        let back: Task = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(t, back);
    }

    #[test]
    fn kind_rename() {
        #[derive(Deserialize)]
        struct Shadow {
            kind: TaskKind,
        }

        let pairs = [
            ("milestone", TaskKind::Milestone),
            ("feature", TaskKind::Feature),
            ("bug", TaskKind::Bug),
            ("task", TaskKind::Task),
            ("chore", TaskKind::Chore),
            ("spike", TaskKind::Spike),
            ("decision", TaskKind::Decision),
        ];
        for (s, expected) in pairs {
            let v: Shadow = serde_json::from_str(&format!(r#"{{"kind":"{s}"}}"#))
                .unwrap_or_else(|_| panic!("deserialize {s}"));
            assert_eq!(v.kind, expected);
        }
    }

    #[test]
    fn status_rename() {
        #[derive(Deserialize)]
        struct Shadow {
            status: TaskStatus,
        }

        let v: Shadow = serde_json::from_str(r#"{"status":"open"}"#).expect("open");
        assert_eq!(v.status, TaskStatus::Open);

        let v: Shadow = serde_json::from_str(r#"{"status":"in_progress"}"#).expect("in_progress");
        assert_eq!(v.status, TaskStatus::InProgress);

        let v: Shadow = serde_json::from_str(r#"{"status":"done"}"#).expect("done");
        assert_eq!(v.status, TaskStatus::Done);

        let v: Shadow = serde_json::from_str(r#"{"status":"failed"}"#).expect("failed");
        assert_eq!(v.status, TaskStatus::Failed);
    }

    #[test]
    fn invalid_json_returns_err() {
        let r: Result<Task, _> = serde_json::from_str(r#"{"id":42}"#);
        assert!(r.is_err());

        let r: Result<TaskKind, _> = serde_json::from_str(r#""bogus_kind""#);
        assert!(r.is_ok()); // unknown kinds map to Unknown variant (backward compat)
        assert_eq!(r.unwrap(), TaskKind::Unknown);

        let r: Result<TaskStatus, _> = serde_json::from_str(r#""unknown""#);
        assert!(r.is_err());
    }

    #[test]
    fn priority_defaults_to_p2_when_absent() {
        // Old JSONL without a priority field deserializes as P2.
        let json = r#"{"id":"old","title":"Old","kind":"task","status":"open","parent":null,"deps":[],"description":"","acceptance":"","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}"#;
        let t: Task = serde_json::from_str(json).expect("deserialize old format");
        assert_eq!(t.priority, 2);
    }

    #[test]
    fn attempts_defaults_to_zero_when_absent() {
        // Old JSONL without an attempts field (#47 backward compat).
        let json = r#"{"id":"old","title":"Old","kind":"task","status":"open","parent":null,"deps":[],"description":"","acceptance":"","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}"#;
        let t: Task = serde_json::from_str(json).expect("deserialize old format");
        assert_eq!(t.attempts, 0);
    }

    #[test]
    fn priority_roundtrip() {
        let t = Task {
            id: "p0".into(),
            title: "Urgent".into(),
            kind: TaskKind::Bug,
            status: TaskStatus::Open,
            attempts: 0,
            priority: 0,
            parent: None,
            deps: vec![],
            description: String::new(),
            acceptance: String::new(),
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
        };
        let json = serde_json::to_string(&t).expect("serialize");
        assert!(json.contains(r#""priority":0"#));
        let back: Task = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(t, back);
    }
}
