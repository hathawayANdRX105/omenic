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

mod defaults;

use std::path::Path;

use serde::Deserialize;

use crate::store::Store;
use crate::{Task, TaskKind, TaskStatus};

pub use defaults::DEFAULT_TEMPLATES;

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
    /// Step keys that must be present in tasks. If any is missing, `apply`
    /// returns an error. Declared via `mandatory:` in the YAML.
    pub mandatory: Vec<String>,
}

// --- YAML shapes -----------------------------------------------------------
#[derive(Deserialize)]
struct YamlTemplate {
    tasks: Vec<YamlTask>,
    #[serde(default)]
    deps: Vec<YamlDep>,
    #[serde(default)]
    mandatory: Vec<String>,
}

#[derive(Deserialize)]
struct YamlTask {
    key: String,
    title: String,
    #[serde(default)]
    #[allow(dead_code)] // 只为兼容既有 yaml schema 而反序列化, kind 实际取自模板
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
        mandatory: yaml.mandatory,
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

    // Mandatory phase check: if the template declares mandatory keys, all
    // must be present in the tasks. This enforces required phases at apply
    // time, preventing agents from skipping plan/implement/audit/smoke.
    if !tpl.mandatory.is_empty() {
        let task_keys: std::collections::HashSet<&str> =
            tpl.tasks.iter().map(|t| t.key.as_str()).collect();
        let missing: Vec<&str> = tpl
            .mandatory
            .iter()
            .filter(|k| !task_keys.contains(k.as_str()))
            .map(|s| s.as_str())
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "template `{name}` is missing mandatory phase(s): {}",
                missing.join(", ")
            ));
        }
    }

    // Topic task: reuse if exists, else create (kind=feature).
    let all = store.load_all().map_err(|e| format!("store error: {e}"))?;
    let exists = all.iter().any(|t| t.id == topic);
    let mut created = Vec::new();
    if !exists {
        let now = crate::now_iso();
        let topic_task = Task {
            id: topic.to_string(),
            title: topic.to_string(),
            kind: TaskKind::Feature,
            status: TaskStatus::Open,
            attempts: 0,
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
        let now = crate::now_iso();
        let task = Task {
            id: id.clone(),
            title: t.title.clone(),
            kind: TaskKind::Task,
            status: TaskStatus::Open,
            attempts: 0,
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
        let now = crate::now_iso();
        let phase_task = Task {
            id: pid.clone(),
            title: format!("{name}: {topic}"),
            kind: TaskKind::Task,
            status: TaskStatus::Open,
            attempts: 0,
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

    // Dep edges: explicit template deps, then two #216 automatic edges:
    // 1) sibling chain — same-parent tasks in declaration order get
    //    `later depends_on earlier` when no explicit edge exists;
    // 2) parent aggregation — the phase task depends on every terminal step
    //    (a step no other step depends on), so the phase closes only after
    //    all its steps, even when the template omits a `phase` dep entry.
    let mut explicit: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for (task_key, dep_key) in &tpl.deps {
        let tid = id_of
            .get(task_key)
            .ok_or_else(|| format!("template dep task `{task_key}` not generated"))?;
        let did = id_of
            .get(dep_key)
            .ok_or_else(|| format!("template dep target `{dep_key}` not generated"))?;
        if tid != did {
            explicit.insert((tid.clone(), did.clone()));
        }
    }

    // Sibling chain: ordered step ids (declaration order).
    let step_ids: Vec<String> = tpl
        .tasks
        .iter()
        .filter(|t| t.key != "phase")
        .filter_map(|t| id_of.get(&t.key).cloned())
        .collect();
    for pair in step_ids.windows(2) {
        let (prev, next) = (&pair[0], &pair[1]);
        if !explicit.contains(&(next.clone(), prev.clone()))
            && !explicit.contains(&(prev.clone(), next.clone()))
        {
            explicit.insert((next.clone(), prev.clone()));
        }
    }

    // Parent aggregation: phase depends on every terminal step. Terminal =
    // a step that no OTHER step depends on (explicit or auto-chain edges).
    let phase_id_str = phase_id.clone().unwrap_or_else(|| topic.to_string());
    let step_depended: std::collections::HashSet<String> = explicit
        .iter()
        .filter(|(_, did)| did != &phase_id_str)
        .map(|(_, did)| did.clone())
        .collect();
    for t in &tpl.tasks {
        if t.key == "phase" {
            continue;
        }
        if let Some(sid) = id_of.get(&t.key)
            && !step_depended.contains(sid)
        {
            explicit.insert((phase_id_str.clone(), sid.clone()));
        }
    }

    let mut task_list = store.load_all().map_err(|e| format!("store error: {e}"))?;
    for (tid, did) in &explicit {
        if let Some(t) = task_list.iter_mut().find(|t| &t.id == tid)
            && !t.deps.contains(did)
        {
            t.deps.push(did.clone());
        }
    }
    for t in task_list {
        store.append(&t).map_err(|e| format!("store error: {e}"))?;
    }

    Ok(created)
}
