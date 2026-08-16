//! Task templates: phases + steps as YAML files under `<data_dir>/templates/`.
//!
//! Structure follows compass-ws `config/templates/{phases,steps}/*.yaml`
//! (compass-specific fields like `refs` are ignored):
//!
//! ```yaml
//! tasks:
//!   - key: scope
//!     title: "scope: define topic in and out"
//!     kind: chore
//!     description: "..."
//!     acceptance: "..."
//! deps:
//!   - task: choose-template
//!     depends_on: scope
//! ```
//!
//! A `phase` template (from `templates/phases/`) applies as
//! topic → phase task → step tasks (1→n→m); a `step` template (from
//! `templates/steps/`) applies as a single task under the topic. The task
//! whose key is `phase` is the phase entry: it becomes the phase task itself
//! (not a separate step) and, when listed in `deps`, its parent-aggregation
//! edge (phase depends on its steps) is wired automatically.

use std::path::Path;

use serde::Deserialize;

use crate::store::Store;
use crate::task::{Task, TaskKind, TaskStatus};

/// Template kind: phase (topic → phase → steps) or step (single task).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TemplateKind {
    Phase,
    Step,
}

/// One task definition inside a template.
#[derive(Debug, Clone)]
pub struct TaskDef {
    pub key: String,
    pub title: String,
    pub description: String,
    pub acceptance: String,
}

/// A parsed template.
#[derive(Debug, Clone)]
pub struct TemplateDef {
    pub name: String,
    pub kind: TemplateKind,
    pub tasks: Vec<TaskDef>,
    /// (task_key, depends_on_key) edges.
    pub deps: Vec<(String, String)>,
}

// --- YAML shapes -----------------------------------------------------------

#[derive(Deserialize)]
struct YamlTemplate {
    tasks: Vec<YamlTask>,
    #[serde(default)]
    deps: Vec<YamlDep>,
}

#[derive(Deserialize)]
struct YamlTask {
    key: String,
    title: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    description: String,
    #[serde(default)]
    acceptance: String,
}

#[derive(Deserialize)]
struct YamlDep {
    task: String,
    depends_on: String,
}

// --- Default templates (written by `oi init`, user-editable) ---------------

/// (kind, name, yaml content). `dev` / `plan` keep the historical oi step
/// chains; `scheme` / `capability` come from compass (cx-only prose trimmed).
pub const DEFAULT_TEMPLATES: &[(TemplateKind, &str, &str)] = &[
    (
        TemplateKind::Phase,
        "dev",
        r#"tasks:
  - key: implement
    title: "implement: deliver the work item"
    kind: task
    description: |
      实现工作项：模块/测试/CLI 路径；主路径跑通后至少一次 smoke。
    acceptance: |
      工作项已实现：模块/测试/CLI 路径 + smoke 通过。
  - key: verify
    title: "verify: check observable contract"
    kind: task
    description: |
      验证可观察契约：focused test + smoke + diff check。
    acceptance: |
      可观察契约验证通过：focused test + smoke + diff check。
  - key: review
    title: "review: scope/CRG/code/simplicity"
    kind: task
    description: |
      审查：scope/CRG/code/simplicity 四面；P0/P1 全部处置。
    acceptance: |
      审查完成：P0/P1 全部 disposition。
  - key: document
    title: "document: sync design/manual"
    kind: task
    description: |
      同步设计文档/功能文档/手册；只更新真正改动的。
    acceptance: |
      设计文档/功能文档/手册已同步。
  - key: tidy
    title: "tidy: clean obsolete artifacts"
    kind: task
    description: |
      清理脚手架/死代码/过期注释。
    acceptance: |
      脚手架/死代码已清理。
  - key: handoff
    title: "handoff: record evidence + next steps"
    kind: task
    description: |
      记录证据 + 下一步命令，新 agent 可接手。
    acceptance: |
      证据 + 下一步命令已记录。
deps:
  - task: verify
    depends_on: implement
  - task: review
    depends_on: verify
  - task: document
    depends_on: review
  - task: tidy
    depends_on: document
  - task: handoff
    depends_on: tidy
"#,
    ),
    (
        TemplateKind::Phase,
        "scheme",
        r#"tasks:
  - key: phase
    title: "scheme: plan-phase with discussion chain"
    kind: task
    description: |
      Plan-phase 编排例（路径未定）：scope → options → feasibility →
      approach → ready-summary → approval 全套讨论链。approval 是人工门。
    acceptance: |
      方案锁定，ready-for-dev: yes；approval 人工门已过。
  - key: scope
    title: "scope: define topic in and out"
    kind: task
    description: |
      定义 topic 的可观察结果、明确的 out-of-scope 边界与归属 milestone。
    acceptance: |
      description 与 acceptance 识别一个独立可评审的 topic。
  - key: options
    title: "options: 2-3 approaches with tradeoffs"
    kind: task
    description: |
      列出 2-3 个方案与 tradeoffs。
    acceptance: |
      至少 2 个方案 + tradeoffs 已列出。
  - key: feasibility
    title: "feasibility: 8-point checklist"
    kind: task
    description: |
      8 项检查表：边界/回滚/可测/兼容/安全/性能/文档/工具链。
    acceptance: |
      8 项检查表完成。
  - key: approach
    title: "approach: lock stack and change path"
    kind: task
    description: |
      锁定方案：技术栈 + 改动路径。
    acceptance: |
      方案锁定：技术栈 + 改动路径。
  - key: ready-summary
    title: "ready-summary: approvable execution summary"
    kind: task
    description: |
      可批准的执行摘要 + Work-items 列表 + Implement-terminal。
    acceptance: |
      执行摘要 + Work-items + Implement-terminal 已产出。
  - key: approval
    title: "approval: human gate"
    kind: task
    description: |
      人工审批门（agent 不自动关闭）。
    acceptance: |
      人工审批通过（approval: yes）。
deps:
  - task: options
    depends_on: scope
  - task: feasibility
    depends_on: options
  - task: approach
    depends_on: feasibility
  - task: ready-summary
    depends_on: approach
  - task: approval
    depends_on: ready-summary
  - task: phase
    depends_on: approval
"#,
    ),
    (
        TemplateKind::Phase,
        "capability",
        r#"tasks:
  - key: phase
    title: "capability: plan-phase with fixed approach"
    kind: task
    description: |
      Plan-phase 编排例（路径已固定）：无讨论链，只选/套/接模板。
    acceptance: |
      模板应用完成；entry-dep matrix 检查过；capability（非 scheme）理由记录。
  - key: scope
    title: "scope: define topic in and out"
    kind: task
    description: |
      定义 topic 的可观察结果与边界。
    acceptance: |
      description 与 acceptance 识别一个独立可评审的 topic。
  - key: choose-template
    title: "choose: select step recipes"
    kind: task
    description: |
      只选匹配 topic 的 step 模板，各挂到真实 phase 下。
    acceptance: |
      每个选中模板记录 category/name/phase parent/理由。
  - key: apply-template
    title: "apply: attach selected step recipes"
    kind: task
    description: |
      应用每个选中的 step；幂等（已存在则记录 already-applied）。
    acceptance: |
      每个 recipe 记录 applied|already-applied。
  - key: shape-graph
    title: "shape: set phases and real dependency gates"
    kind: task
    description: |
      加真实 dep 边：implement entry -dep approval（如有）；verify/review -dep
      Implement-terminal；entry-dep matrix 检查。
    acceptance: |
      entry-dep matrix 检查通过；plan/ready/blocked 与意图一致。
deps:
  - task: choose-template
    depends_on: scope
  - task: apply-template
    depends_on: choose-template
  - task: shape-graph
    depends_on: apply-template
  - task: phase
    depends_on: shape-graph
"#,
    ),
    (
        TemplateKind::Phase,
        "pdca",
        r#"tasks:
  - key: phase
    title: "pdca: plan → implement → audit → smoke → tidy"
    kind: task
    description: |
      PDCA 编排 phase：plan → implement → audit → smoke → tidy 顺序链。
    acceptance: |
      PDCA 五步全部完成；phase 在 tidy 完成后关闭。
  - key: plan
    title: "plan: define approach and steps"
    kind: task
    description: |
      定义方案与实施步骤（同层第一步）。
    acceptance: |
      方案与步骤已定义。
  - key: implement
    title: "implement: deliver the work item"
    kind: task
    description: |
      按方案实现：模块/测试/CLI 路径。
    acceptance: |
      工作项已实现。
  - key: audit
    title: "audit: check against plan and contract"
    kind: task
    description: |
      对照方案与可观察契约审查实现。
    acceptance: |
      审计完成：实现与方案/契约一致。
  - key: smoke
    title: "smoke: run and observe"
    kind: task
    description: |
      跑通主路径并记录观察结果。
    acceptance: |
      smoke 通过：命令 + 结果已记录。
  - key: tidy
    title: "tidy: clean obsolete artifacts"
    kind: task
    description: |
      清理脚手架/死代码/过期注释。
    acceptance: |
      清理完成。
deps:
  - task: implement
    depends_on: plan
  - task: audit
    depends_on: implement
  - task: smoke
    depends_on: audit
  - task: tidy
    depends_on: smoke
  - task: phase
    depends_on: tidy
"#,
    ),
    (
        TemplateKind::Phase,
        "plan",
        r#"tasks:
  - key: scope
    title: "scope: define topic in and out"
    kind: task
    description: |
      定义 In/Out/可观察结果/Non-goals。
    acceptance: |
      In/Out/可观察结果/Non-goals 已定义。
  - key: options
    title: "options: 2-3 approaches with tradeoffs"
    kind: task
    description: |
      2-3 个方案 + tradeoffs。
    acceptance: |
      2-3 个方案 + tradeoffs 已列出。
  - key: feasibility
    title: "feasibility: 8-point checklist"
    kind: task
    description: |
      8 项检查表（边界/回滚/可测/兼容/安全/性能/文档/工具链）。
    acceptance: |
      8 项检查表完成。
  - key: approach
    title: "approach: lock stack and change path"
    kind: task
    description: |
      锁定方案：技术栈 + 改动路径。
    acceptance: |
      方案锁定：技术栈 + 改动路径。
  - key: ready-summary
    title: "ready-summary: approvable execution summary"
    kind: task
    description: |
      可批准的执行摘要 + Work-items + Implement-terminal。
    acceptance: |
      执行摘要 + Work-items + Implement-terminal 已产出。
  - key: approval
    title: "approval: human gate"
    kind: task
    description: |
      人工审批门。
    acceptance: |
      人工审批通过（approval: yes）。
deps:
  - task: options
    depends_on: scope
  - task: feasibility
    depends_on: options
  - task: approach
    depends_on: feasibility
  - task: ready-summary
    depends_on: approach
  - task: approval
    depends_on: ready-summary
"#,
    ),
    (
        TemplateKind::Step,
        "implement",
        r#"tasks:
  - key: implement
    title: "implement: deliver the work item"
    kind: task
    description: |
      实现工作项：模块/测试/CLI 路径；至少一次 smoke。
    acceptance: |
      工作项已实现：模块/测试/CLI 路径 + smoke 通过。
deps: []
"#,
    ),
    (
        TemplateKind::Step,
        "verify",
        r#"tasks:
  - key: verify
    title: "verify: check observable contract"
    kind: task
    description: |
      验证可观察契约：focused test + smoke + diff check。
    acceptance: |
      可观察契约验证通过。
deps: []
"#,
    ),
    (
        TemplateKind::Step,
        "review",
        r#"tasks:
  - key: review
    title: "review: scope/CRG/code/simplicity"
    kind: task
    description: |
      审查：scope/CRG/code/simplicity；P0/P1 全部处置。
    acceptance: |
      审查完成：P0/P1 全部 disposition。
deps: []
"#,
    ),
    (
        TemplateKind::Step,
        "document",
        r#"tasks:
  - key: document
    title: "document: sync design/manual"
    kind: task
    description: |
      同步设计文档/功能文档/手册。
    acceptance: |
      文档已同步。
deps: []
"#,
    ),
    (
        TemplateKind::Step,
        "tidy",
        r#"tasks:
  - key: tidy
    title: "tidy: clean obsolete artifacts"
    kind: task
    description: |
      清理脚手架/死代码/过期注释。
    acceptance: |
      脚手架/死代码已清理。
deps: []
"#,
    ),
];

/// Write default templates into `<dir>/templates/{phases,steps}/` —
/// idempotent, never overwrites existing files.
pub fn write_default_templates(dir: &Path) -> Result<(), String> {
    for (kind, name, content) in DEFAULT_TEMPLATES {
        let sub = match kind {
            TemplateKind::Phase => "phases",
            TemplateKind::Step => "steps",
        };
        let tdir = dir.join("templates").join(sub);
        std::fs::create_dir_all(&tdir).map_err(|e| format!("create {}: {e}", tdir.display()))?;
        let path = tdir.join(format!("{name}.yaml"));
        if !path.exists() {
            std::fs::write(&path, content)
                .map_err(|e| format!("write template {}: {e}", path.display()))?;
        }
    }
    Ok(())
}

/// Parse a template YAML document.
pub fn parse_template(
    name: &str,
    kind: TemplateKind,
    content: &str,
) -> Result<TemplateDef, String> {
    let yaml: YamlTemplate =
        serde_yaml::from_str(content).map_err(|e| format!("parse template `{name}`: {e}"))?;
    if yaml.tasks.is_empty() {
        return Err(format!("template `{name}` has no tasks"));
    }
    let tasks = yaml
        .tasks
        .into_iter()
        .map(|t| TaskDef {
            key: t.key,
            title: t.title,
            description: t.description,
            acceptance: t.acceptance,
        })
        .collect();
    let deps = yaml
        .deps
        .into_iter()
        .map(|d| (d.task, d.depends_on))
        .collect();
    Ok(TemplateDef {
        name: name.to_string(),
        kind,
        tasks,
        deps,
    })
}

/// Look up a template file: `phases/<name>.yaml` then `steps/<name>.yaml`.
pub fn load_template(dir: &Path, name: &str) -> Result<TemplateDef, String> {
    for (sub, kind) in [
        ("phases", TemplateKind::Phase),
        ("steps", TemplateKind::Step),
    ] {
        let path = dir.join("templates").join(sub).join(format!("{name}.yaml"));
        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| format!("read {}: {e}", path.display()))?;
            return parse_template(name, kind, &content);
        }
    }
    Err(format!(
        "template `{name}` not found in {} (run `oi init` or add the file)",
        dir.join("templates").display()
    ))
}

/// List all templates from `templates/phases/` and `templates/steps/`.
pub fn load_all_templates(dir: &Path) -> Result<Vec<TemplateDef>, String> {
    let mut out = Vec::new();
    for (sub, kind) in [
        ("phases", TemplateKind::Phase),
        ("steps", TemplateKind::Step),
    ] {
        let tdir = dir.join("templates").join(sub);
        if !tdir.is_dir() {
            continue;
        }
        let mut entries: Vec<_> = std::fs::read_dir(&tdir)
            .map_err(|e| format!("read {}: {e}", tdir.display()))?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|x| x == "yaml").unwrap_or(false))
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let content = std::fs::read_to_string(&path)
                .map_err(|e| format!("read {}: {e}", path.display()))?;
            match parse_template(&name, kind, &content) {
                Ok(t) => out.push(t),
                Err(e) => return Err(format!("{}: {e}", path.display())),
            }
        }
    }
    Ok(out)
}

/// Apply a template under `topic`: create the topic task if missing, then
/// (for phase templates) a phase task + its step tasks; for step templates a
/// single task. Dep edges from the template map onto generated ids; a task
/// whose key is `phase` is the phase task itself, and a `phase` entry in
/// `deps` wires parent-aggregation (phase depends on its steps).
pub fn apply(
    store: &Store,
    dir: &Path,
    name: &str,
    topic: &str,
    parent: Option<String>,
) -> Result<Vec<String>, String> {
    let tpl = load_template(dir, name)?;

    // Topic task: reuse if exists, else create (kind=feature).
    let all = store.load_all().map_err(|e| format!("store error: {e}"))?;
    let exists = all.iter().any(|t| t.id == topic);
    let mut created = Vec::new();
    if !exists {
        let now = crate::task::now_iso();
        let topic_task = Task {
            id: topic.to_string(),
            title: topic.to_string(),
            kind: TaskKind::Feature,
            status: TaskStatus::Open,
            priority: 2,
            parent: parent.clone(),
            deps: vec![],
            description: String::new(),
            acceptance: String::new(),
            created_at: now.clone(),
            updated_at: now,
        };
        store
            .append(&topic_task)
            .map_err(|e| format!("store error: {e}"))?;
        created.push(topic.to_string());
    }

    // Generate tasks: phase entry (key=phase) becomes the phase task; all
    // other tasks are steps under it (phase templates) or under topic (step
    // templates).
    let phase_id = match tpl.kind {
        TemplateKind::Phase => Some(format!("{topic}-{name}")),
        TemplateKind::Step => None,
    };
    let mut id_of: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for t in &tpl.tasks {
        if t.key == "phase" {
            // Phase entry task: its title/acceptance describe the phase itself.
            id_of.insert(
                t.key.clone(),
                phase_id.clone().unwrap_or_else(|| topic.to_string()),
            );
            continue;
        }
        let id = match &phase_id {
            Some(p) => format!("{p}-{}", t.key),
            None => format!("{topic}-{}", t.key),
        };
        let now = crate::task::now_iso();
        let task = Task {
            id: id.clone(),
            title: t.title.clone(),
            kind: TaskKind::Task,
            status: TaskStatus::Open,
            priority: 2,
            parent: phase_id.clone().or_else(|| Some(topic.to_string())),
            deps: vec![],
            description: t.description.clone(),
            acceptance: t.acceptance.clone(),
            created_at: now.clone(),
            updated_at: now,
        };
        store
            .append(&task)
            .map_err(|e| format!("store error: {e}"))?;
        id_of.insert(t.key.clone(), id.clone());
        created.push(id);
    }

    // Phase task itself (if the template had no `phase` entry, synthesize one
    // so the tree is topic → phase → steps).
    if let Some(pid) = &phase_id
        && !all.iter().any(|t| t.id == *pid)
    {
        let now = crate::task::now_iso();
        let phase_task = Task {
            id: pid.clone(),
            title: format!("{name}: {topic}"),
            kind: TaskKind::Task,
            status: TaskStatus::Open,
            priority: 2,
            parent: Some(topic.to_string()),
            deps: vec![],
            description: format!("编排阶段 `{name}`（模板：{name}）"),
            acceptance: String::new(),
            created_at: now.clone(),
            updated_at: now,
        };
        store
            .append(&phase_task)
            .map_err(|e| format!("store error: {e}"))?;
        created.push(pid.clone());
        id_of.insert("phase".to_string(), pid.clone());
    }

    // Dep edges: map keys → ids. Unknown keys are skipped (warned by caller
    // not possible here — keep strict: error out).
    let mut task_list = store.load_all().map_err(|e| format!("store error: {e}"))?;
    for (task_key, dep_key) in &tpl.deps {
        let tid = id_of
            .get(task_key)
            .ok_or_else(|| format!("template dep task `{task_key}` not generated"))?;
        let did = id_of
            .get(dep_key)
            .ok_or_else(|| format!("template dep target `{dep_key}` not generated"))?;
        if tid == did {
            continue;
        }
        if let Some(t) = task_list.iter_mut().find(|t| &t.id == tid) {
            if !t.deps.contains(did) {
                t.deps.push(did.clone());
            }
        }
    }
    for t in task_list {
        store.append(&t).map_err(|e| format!("store error: {e}"))?;
    }

    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn tmp_store(dir: &Path) -> Store {
        Store::new(dir)
    }

    #[test]
    fn parse_default_templates() {
        for &(kind, name, content) in DEFAULT_TEMPLATES {
            let t = parse_template(name, kind, content).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!t.tasks.is_empty(), "{name}: no tasks");
            assert_eq!(t.name.as_str(), name);
        }
    }

    #[test]
    fn scheme_has_approval_and_phase_aggregation() {
        let t = parse_template(
            "scheme",
            TemplateKind::Phase,
            DEFAULT_TEMPLATES
                .iter()
                .find(|(_, n, _)| *n == "scheme")
                .unwrap()
                .2,
        )
        .unwrap();
        assert!(t.tasks.iter().any(|x| x.key == "approval"));
        assert!(
            t.deps
                .iter()
                .any(|(task, dep)| task == "phase" && dep == "approval")
        );
    }

    #[test]
    fn apply_phase_creates_topic_phase_steps() {
        let tmp = tempdir().unwrap();
        let store = tmp_store(tmp.path());
        write_default_templates(tmp.path()).unwrap();
        let ids = apply(&store, tmp.path(), "scheme", "auth-flow", None).unwrap();
        // topic + phase + 6 steps (scope..approval; phase key merged into phase task)
        assert_eq!(ids.len(), 1 + 1 + 6, "{ids:?}");

        let all = store.load_all().unwrap();
        let map: std::collections::HashMap<String, Task> =
            all.into_iter().map(|t| (t.id.clone(), t)).collect();

        let phase = &map["auth-flow-scheme"];
        assert_eq!(phase.parent.as_deref(), Some("auth-flow"));
        assert_eq!(phase.title, "scheme: auth-flow");

        let scope = &map["auth-flow-scheme-scope"];
        assert_eq!(scope.parent.as_deref(), Some("auth-flow-scheme"));
        assert!(scope.deps.is_empty(), "first step should have no deps");

        let options = &map["auth-flow-scheme-options"];
        assert_eq!(options.deps, vec!["auth-flow-scheme-scope".to_string()]);

        // Phase aggregates its steps: phase depends on approval (last step).
        assert_eq!(
            phase.deps,
            vec!["auth-flow-scheme-approval".to_string()],
            "phase must depend on its terminal step"
        );
    }

    #[test]
    fn pdca_template_has_five_step_chain_and_phase_aggregation() {
        let t = parse_template(
            "pdca",
            TemplateKind::Phase,
            DEFAULT_TEMPLATES
                .iter()
                .find(|(_, n, _)| *n == "pdca")
                .unwrap()
                .2,
        )
        .unwrap();
        let keys: Vec<&str> = t.tasks.iter().map(|x| x.key.as_str()).collect();
        assert_eq!(
            keys,
            vec!["phase", "plan", "implement", "audit", "smoke", "tidy"]
        );
        // Sibling chain derived automatically even with explicit deps present.
        let apply_result = {
            let tmp = tempdir().unwrap();
            let store = tmp_store(tmp.path());
            write_default_templates(tmp.path()).unwrap();
            apply(&store, tmp.path(), "pdca", "t-pd", None).unwrap()
        };
        assert_eq!(apply_result.len(), 1 + 1 + 5, "{apply_result:?}");
    }
    #[test]
    fn apply_step_creates_single_task_under_topic() {
        let tmp = tempdir().unwrap();
        let store = tmp_store(tmp.path());
        write_default_templates(tmp.path()).unwrap();
        let ids = apply(&store, tmp.path(), "verify", "sub-x", None).unwrap();
        assert_eq!(ids.len(), 2, "{ids:?}"); // topic + verify
        let all = store.load_all().unwrap();
        let map: std::collections::HashMap<String, Task> =
            all.into_iter().map(|t| (t.id.clone(), t)).collect();
        let v = &map["sub-x-verify"];
        assert_eq!(v.parent.as_deref(), Some("sub-x"));
        assert!(v.deps.is_empty());
    }

    #[test]
    fn apply_reuses_existing_topic() {
        let tmp = tempdir().unwrap();
        let store = tmp_store(tmp.path());
        write_default_templates(tmp.path()).unwrap();
        // Create topic first via a step template, then apply a phase on it.
        apply(&store, tmp.path(), "verify", "sub-x", None).unwrap();
        let ids = apply(&store, tmp.path(), "dev", "sub-x", None).unwrap();
        assert!(
            !ids.contains(&"sub-x".to_string()),
            "topic must not be recreated"
        );
        let all = store.load_all().unwrap();
        assert!(all.iter().any(|t| t.id == "sub-x-dev"));
    }
}
