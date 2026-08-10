//! Core task types: `Task`, `TaskKind`, `TaskStatus`.
//!
//! Serde-backed persistence model used as the foundation for store and graph layers.

use serde::{Deserialize, Serialize};

/// Whether a task is a plain task, a dependency-only template, or a reusable template.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Task,
    Dep,
    Template,
}

/// Progress state of a task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Open,
    InProgress,
    Done,
}

/// A single work item in the task graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub kind: TaskKind,
    pub status: TaskStatus,
    pub parent: Option<String>,
    pub deps: Vec<String>,
    pub description: String,
    pub acceptance: String,
    pub created_at: String,
    pub updated_at: String,
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
    }

    #[test]
    fn roundtrip() {
        let t = Task {
            id: "roundtrip".into(),
            title: "Round Trip".into(),
            kind: TaskKind::Dep,
            status: TaskStatus::Done,
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

        let v: Shadow = serde_json::from_str(r#"{"kind":"task"}"#).expect("task");
        assert_eq!(v.kind, TaskKind::Task);

        let v: Shadow = serde_json::from_str(r#"{"kind":"dep"}"#).expect("dep");
        assert_eq!(v.kind, TaskKind::Dep);

        let v: Shadow = serde_json::from_str(r#"{"kind":"template"}"#).expect("template");
        assert_eq!(v.kind, TaskKind::Template);
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
    }

    #[test]
    fn invalid_json_returns_err() {
        let r: Result<Task, _> = serde_json::from_str(r#"{"id":42}"#);
        assert!(r.is_err());

        let r: Result<TaskKind, _> = serde_json::from_str(r#""bogus_kind""#);
        assert!(r.is_err());

        let r: Result<TaskStatus, _> = serde_json::from_str(r#""unknown""#);
        assert!(r.is_err());
    }
}
