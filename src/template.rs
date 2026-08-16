//! Task templates: reusable step chains for orchestration.
//!
//! Aligns with `todo/spike/oi-task-handbook.md` §2: a phase is a chain of
//! atomic steps with internal deps. MVP ships the two phase templates as
//! built-in constants; file-backed templates are a 1→n enhancement
//! (`todo/spike/design-1toN.md` §3.2).

use crate::store::Store;
use crate::task::{Task, TaskKind, TaskStatus};

/// One step in a template chain.
pub struct StepDef {
    /// kebab suffix used to build the step task id: `<topic>-<name>`.
    pub name: &'static str,
    /// Display title of the step task.
    pub title: &'static str,
    /// Acceptance criteria seeded into the step task.
    pub acceptance: &'static str,
}

/// A named template: one parent (topic) task + a sequential step chain.
pub struct Template {
    pub name: &'static str,
    pub description: &'static str,
    pub steps: &'static [StepDef],
}

/// Built-in templates. First step has no deps; each later step depends on
/// its predecessor, so the chain runs strictly in order.
pub const TEMPLATES: &[Template] = &[
    Template {
        name: "dev",
        description: "implement → verify → review → document → tidy → handoff",
        steps: &[
            StepDef {
                name: "implement",
                title: "implement",
                acceptance: "工作项已实现：模块/测试/CLI 路径 + smoke 通过",
            },
            StepDef {
                name: "verify",
                title: "verify",
                acceptance: "可观察契约验证通过：focused test + smoke + diff check",
            },
            StepDef {
                name: "review",
                title: "review",
                acceptance: "审查完成：scope/CRG/code/simplicity，P0/P1 全部 disposition",
            },
            StepDef {
                name: "document",
                title: "document",
                acceptance: "设计文档/功能文档/手册已同步",
            },
            StepDef {
                name: "tidy",
                title: "tidy",
                acceptance: "脚手架/死代码已清理（可选）",
            },
            StepDef {
                name: "handoff",
                title: "handoff",
                acceptance: "证据 + 下一步命令已记录，新 agent 可接手",
            },
        ],
    },
    Template {
        name: "plan",
        description: "scope → options → feasibility → approach → ready-summary → approval",
        steps: &[
            StepDef {
                name: "scope",
                title: "scope",
                acceptance: "In/Out/可观察结果/Non-goals 已定义",
            },
            StepDef {
                name: "options",
                title: "options",
                acceptance: "2-3 个方案 + tradeoffs 已列出",
            },
            StepDef {
                name: "feasibility",
                title: "feasibility",
                acceptance: "8 项检查表完成（边界/回滚/可测/兼容/安全/性能/文档/工具链）",
            },
            StepDef {
                name: "approach",
                title: "approach",
                acceptance: "方案锁定：技术栈 + 改动路径",
            },
            StepDef {
                name: "ready-summary",
                title: "ready-summary",
                acceptance: "可批准的执行摘要 + Work-items 列表 + Implement-terminal",
            },
            StepDef {
                name: "approval",
                title: "approval",
                acceptance: "人工审批门（agent 不自动关闭）",
            },
        ],
    },
];

/// Look up a built-in template by name.
pub fn find(name: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|t| t.name == name)
}

/// Apply a template: create the topic task, then each step task as a child
/// with a dependency chain through the previous step. Returns created ids in
/// order (topic first). Ids use the existing oi convention (title as id);
/// step ids are `<topic>-<step-name>`.
pub fn apply(
    store: &Store,
    tpl: &'static Template,
    topic: &str,
    parent: Option<String>,
) -> Result<Vec<String>, String> {
    if topic.trim().is_empty() {
        return Err("template apply requires a topic title".to_string());
    }
    let all = store.load_all().map_err(|e| format!("store error: {e}"))?;
    let existing: std::collections::HashSet<&str> = all.iter().map(|t| t.id.as_str()).collect();
    if existing.contains(topic) {
        return Err(format!("task `{topic}` already exists"));
    }

    let now = crate::task::now_iso();
    let topic_task = Task {
        id: topic.to_string(),
        title: topic.to_string(),
        kind: TaskKind::Feature,
        status: TaskStatus::Open,
        priority: 2,
        parent,
        deps: vec![],
        description: String::new(),
        acceptance: String::new(),
        created_at: now.clone(),
        updated_at: now,
    };
    store
        .append(&topic_task)
        .map_err(|e| format!("store error: {e}"))?;

    let mut created = vec![topic.to_string()];
    let mut prev: Option<String> = None;
    for step in tpl.steps {
        let deps = prev.iter().cloned().collect::<Vec<_>>();
        let id = format!("{topic}-{}", step.name);
        let now = crate::task::now_iso();
        let task = Task {
            id: id.clone(),
            title: format!("{}: {}", step.title, topic),
            kind: TaskKind::Task,
            status: TaskStatus::Open,
            priority: 2,
            parent: Some(topic.to_string()),
            deps,
            description: String::new(),
            acceptance: step.acceptance.to_string(),
            created_at: now.clone(),
            updated_at: now,
        };
        store
            .append(&task)
            .map_err(|e| format!("store error: {e}"))?;
        created.push(id.clone());
        prev = Some(id);
    }
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn templates_have_unique_names_and_chain_deps() {
        let mut names = std::collections::HashSet::new();
        for tpl in TEMPLATES {
            assert!(names.insert(tpl.name), "duplicate template {}", tpl.name);
            assert!(!tpl.steps.is_empty());
            for step in tpl.steps {
                assert!(!step.name.is_empty());
                assert!(!step.acceptance.is_empty());
            }
        }
        assert!(find("dev").is_some());
        assert!(find("plan").is_some());
        assert!(find("nope").is_none());
    }

    #[test]
    fn apply_creates_topic_and_ordered_step_chain() {
        let tmp = tempdir().unwrap();
        let store = Store::new(tmp.path());
        let tpl = find("dev").unwrap();
        let ids = apply(&store, tpl, "auth-flow", None).unwrap();
        assert_eq!(ids.len(), 7); // topic + 6 steps
        assert_eq!(ids[0], "auth-flow");

        let all = store.load_all().unwrap();
        let map: std::collections::HashMap<String, Task> =
            all.into_iter().map(|t| (t.id.clone(), t)).collect();

        let topic = &map["auth-flow"];
        assert_eq!(topic.kind, TaskKind::Feature);

        // Steps are children of the topic, chained in order.
        for (i, step) in tpl.steps.iter().enumerate() {
            let id = format!("auth-flow-{}", step.name);
            let t = &map[&id];
            assert_eq!(t.parent.as_deref(), Some("auth-flow"));
            assert_eq!(t.acceptance, step.acceptance);
            let expect_deps: Vec<String> = if i == 0 {
                vec![]
            } else {
                vec![format!("auth-flow-{}", tpl.steps[i - 1].name)]
            };
            assert_eq!(t.deps, expect_deps, "step {id} deps wrong");
        }
    }

    #[test]
    fn apply_rejects_duplicate_topic() {
        let tmp = tempdir().unwrap();
        let store = Store::new(tmp.path());
        let tpl = find("dev").unwrap();
        apply(&store, tpl, "dup-topic", None).unwrap();
        let err = apply(&store, tpl, "dup-topic", None).unwrap_err();
        assert!(err.contains("already exists"));
    }
}
