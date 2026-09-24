//! Task-graph and work-item handlers, split out of `cli.rs`.

use super::*;

use config::Config;
use store::store::Store;
use store::{Task, TaskKind, TaskStatus};

pub fn task_add(
    store: &Store,
    titles: &[String],
    parent: Option<String>,
    deps: Option<String>,
    acceptance: Option<String>,
    priority: Option<u8>,
    kind: Option<String>,
    json: bool,
) -> Result<u8, String> {
    if titles.is_empty() {
        return Err("task add requires at least one title".to_string());
    }
    let deps: Vec<String> = deps
        .map(|d| {
            d.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let acceptance = acceptance.unwrap_or_default();
    let kind = match &kind {
        Some(k) => Some(parse_kind(k)?),
        None => None,
    };

    // Validate deps: existence, no self-dep, no cycle.
    if !deps.is_empty() {
        let all: Vec<Task> = store.load_all().map_err(|e| format!("store error: {e}"))?;
        let id_map: std::collections::HashMap<String, Vec<String>> =
            all.iter().map(|t| (t.id.clone(), t.deps.clone())).collect();
        let existing_ids: std::collections::HashSet<&str> =
            all.iter().map(|t| t.id.as_str()).collect();

        for title in titles {
            for dep in &deps {
                if dep == title {
                    return Err(format!("task `{title}` cannot depend on itself"));
                }
                if !existing_ids.contains(dep.as_str()) {
                    return Err(format!("dependency `{dep}` does not exist"));
                }
                // Simulate adding edge title -> dep and check for cycles.
                let mut sim = id_map.clone();
                sim.entry(title.clone()).or_default().push(dep.clone());
                if store::graph::would_dep_cycle(&sim, title, dep) {
                    return Err(format!(
                        "adding dependency `{title}` -> `{dep}` would create a cycle"
                    ));
                }
            }
        }
    }

    let mut created_ids = Vec::new();
    for title in titles {
        let now = store::now_iso();
        let task = Task {
            id: title.clone(),
            title: title.clone(),
            kind: kind.clone().unwrap_or(TaskKind::Task),
            status: TaskStatus::Open,
            attempts: 0,
            priority: priority.unwrap_or(2),
            parent: parent.clone(),
            deps: deps.clone(),
            description: String::new(),
            acceptance: acceptance.clone(),
            created_at: now.clone(),
            updated_at: now,
        };
        store
            .append(&task)
            .map_err(|e| format!("store error: {e}"))?;
        created_ids.push(task.id);
    }
    if json {
        let msgs: Vec<String> = created_ids
            .iter()
            .map(|id| format!("created {id}"))
            .collect();
        json_ok(&msgs.join("\n"));
    } else {
        for id in &created_ids {
            println!("created {id}");
        }
    }
    Ok(0)
}

pub fn task_done(store: &Store, id: &str, json: bool) -> Result<u8, String> {
    let Some(mut task) = store
        .load_task(id)
        .map_err(|e| format!("store error: {e}"))?
    else {
        eprintln!("task not found: {id}");
        return Ok(1);
    };
    if task.status == TaskStatus::Done {
        eprintln!("task already done: {id}");
        return Ok(1);
    }
    task.status = TaskStatus::Done;
    task.updated_at = store::now_iso();
    store
        .append(&task)
        .map_err(|e| format!("store error: {e}"))?;

    // Suggest newly-unblocked tasks (deps all Done, this task among them, still Open).
    let all = store.load_all().map_err(|e| format!("store error: {e}"))?;
    let ready = suggest_next(&all, id);

    if json {
        let mut obj = serde_json::json!({
            "status": "ok",
            "message": format!("done {id}"),
        });
        if !ready.is_empty() {
            obj["ready"] = serde_json::Value::String(ready.join(", "));
        }
        print_json(&obj);
    } else {
        println!("done {id}");
        if !ready.is_empty() {
            println!("ready: {}", ready.join(", "));
        }
    }
    Ok(0)
}

pub fn task_status(store: &Store, id: &str, json: bool) -> Result<u8, String> {
    let Some(task) = store
        .load_task(id)
        .map_err(|e| format!("store error: {e}"))?
    else {
        eprintln!("task not found: {id}");
        return Ok(1);
    };
    if json {
        print_json(&serde_json::json!({
            "id": task.id,
            "title": task.title,
            "status": format!("{:?}", task.status),
            "priority": task.priority,
            "parent": task.parent,
            "deps": task.deps,
            "created_at": task.created_at,
            "updated_at": task.updated_at,
        }));
    } else {
        println!("id:         {}", task.id);
        println!("title:      {}", task.title);
        println!("status:     {:?}", task.status);
        println!("priority:   P{}", task.priority);
        match &task.parent {
            Some(p) => println!("parent:     {p}"),
            None => println!("parent:     -"),
        }
        if task.deps.is_empty() {
            println!("deps:       -");
        } else {
            println!("deps:       {}", task.deps.join(", "));
        }
        println!("created_at: {}", task.created_at);
        println!("updated_at: {}", task.updated_at);
    }
    Ok(0)
}

/// `dep add <task-id> <dep-id>`: add a dependency edge task-id -> dep-id.
pub fn dep_add(store: &Store, task_id: &str, dep_id: &str, json: bool) -> Result<u8, String> {
    let all = store.load_all().map_err(|e| format!("store error: {e}"))?;

    // Both task-id and dep-id must exist.
    let task = all.iter().find(|t| t.id == task_id).cloned();
    let Some(mut task) = task else {
        eprintln!("task not found: {task_id}");
        return Ok(1);
    };
    if !all.iter().any(|t| t.id == dep_id) {
        eprintln!("task not found: {dep_id}");
        return Ok(1);
    }

    // Self-dependency.
    if task_id == dep_id {
        return Err(format!("task `{task_id}` cannot depend on itself"));
    }

    // Duplicate.
    if task.deps.iter().any(|d| d == dep_id) {
        return Err(format!("dependency already exists: {task_id} -> {dep_id}"));
    }

    // Cycle check: simulate adding the edge.
    let mut sim: std::collections::HashMap<String, Vec<String>> =
        all.iter().map(|t| (t.id.clone(), t.deps.clone())).collect();
    sim.entry(task_id.to_string())
        .or_default()
        .push(dep_id.to_string());
    if store::graph::would_dep_cycle(&sim, task_id, dep_id) {
        return Err(format!(
            "adding dependency `{task_id}` -> `{dep_id}` would create a cycle"
        ));
    }

    task.deps.push(dep_id.to_string());
    task.deps.sort();
    task.updated_at = store::now_iso();
    store
        .append(&task)
        .map_err(|e| format!("store error: {e}"))?;
    let msg = format!("added dependency: {task_id} depends on {dep_id}");
    if json {
        json_ok(&msg);
    } else {
        println!("{msg}");
    }
    Ok(0)
}

/// `dep remove <task-id> <dep-id>`: remove a dependency edge task-id -> dep-id.
pub fn dep_remove(store: &Store, task_id: &str, dep_id: &str, json: bool) -> Result<u8, String> {
    let Some(mut task) = store
        .load_task(task_id)
        .map_err(|e| format!("store error: {e}"))?
    else {
        eprintln!("task not found: {task_id}");
        return Ok(1);
    };

    if !task.deps.iter().any(|d| d == dep_id) {
        return Err(format!("dependency not found: {task_id} -> {dep_id}"));
    }

    task.deps.retain(|d| d != dep_id);
    task.updated_at = store::now_iso();
    store
        .append(&task)
        .map_err(|e| format!("store error: {e}"))?;
    let msg = format!("removed dependency: {task_id} depends on {dep_id}");
    if json {
        json_ok(&msg);
    } else {
        println!("{msg}");
    }
    Ok(0)
}

/// `task update <id> [flags]`
#[allow(clippy::too_many_arguments)]
pub fn task_update(
    store: &Store,
    id: &str,
    title: Option<String>,
    description: Option<String>,
    status: Option<String>,
    deps: Option<String>,
    acceptance: Option<String>,
    priority: Option<u8>,
    kind: Option<String>,
    attempts: Option<u32>,
    json: bool,
) -> Result<u8, String> {
    let Some(mut task) = store
        .load_task(id)
        .map_err(|e| format!("store error: {e}"))?
    else {
        eprintln!("task not found: {id}");
        return Ok(1);
    };

    if let Some(v) = title {
        task.title = v;
    }
    if let Some(v) = description {
        task.description = v;
    }
    if let Some(v) = status {
        task.status = parse_status(&v)?;
    }
    if let Some(v) = deps {
        task.deps = v
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    if let Some(v) = acceptance {
        task.acceptance = v;
    }
    if let Some(p) = priority {
        if p > 4 {
            return Err(format!("invalid priority `{p}` (expected 0-4)"));
        }
        task.priority = p;
    }
    if let Some(v) = kind {
        task.kind = parse_kind(&v)?;
    }
    if let Some(v) = attempts {
        task.attempts = v;
    }

    // Validate deps exist + no cycle when deps changed.
    if !task.deps.is_empty() {
        let all = store.load_all().map_err(|e| format!("store error: {e}"))?;
        let existing_ids: std::collections::HashSet<&str> =
            all.iter().map(|t| t.id.as_str()).collect();
        for dep in &task.deps {
            if dep == id {
                return Err(format!("task `{id}` cannot depend on itself"));
            }
            if !existing_ids.contains(dep.as_str()) {
                return Err(format!("dependency `{dep}` does not exist"));
            }
        }
        let mut sim: std::collections::HashMap<String, Vec<String>> =
            all.iter().map(|t| (t.id.clone(), t.deps.clone())).collect();
        // Replace this task's deps with the new set for the cycle check.
        sim.insert(id.to_string(), task.deps.clone());
        for dep in &task.deps {
            if store::graph::would_dep_cycle(&sim, id, dep) {
                return Err(format!(
                    "adding dependency `{id}` -> `{dep}` would create a cycle"
                ));
            }
        }
    }

    task.updated_at = store::now_iso();
    store
        .append(&task)
        .map_err(|e| format!("store error: {e}"))?;
    let msg = format!("updated {id}");
    if json {
        json_ok(&msg);
    } else {
        println!("{msg}");
    }
    Ok(0)
}

/// `task delete <id>` — delete an isolated task via tombstone.
pub fn task_delete(store: &Store, id: &str, json: bool) -> Result<u8, String> {
    let all = store.load_all().map_err(|e| format!("store error: {e}"))?;
    if !all.iter().any(|t| t.id == id) {
        eprintln!("task not found: {id}");
        return Ok(1);
    }
    let children = store::graph::children_of(&all, id);
    let dependents = store::graph::dependents(&all, id);
    if !children.is_empty() || !dependents.is_empty() {
        let mut reasons = Vec::new();
        if !children.is_empty() {
            reasons.push(format!("children: {}", children.join(", ")));
        }
        if !dependents.is_empty() {
            reasons.push(format!("dependents: {}", dependents.join(", ")));
        }
        return Err(format!(
            "cannot delete: has {reasons}",
            reasons = reasons.join("; ")
        ));
    }
    store
        .append_tombstone(id)
        .map_err(|e| format!("store error: {e}"))?;
    let msg = format!("deleted {id}");
    if json {
        json_ok(&msg);
    } else {
        println!("{msg}");
    }
    Ok(0)
}

/// `task list [--status S] [--kind K] [--parent P]`
pub fn task_list(
    store: &Store,
    status: Option<String>,
    kind: Option<String>,
    parent: Option<String>,
    json: bool,
) -> Result<u8, String> {
    let tasks = store.load_all().map_err(|e| format!("store error: {e}"))?;

    let status_filter: Option<Vec<TaskStatus>> = match status {
        Some(v) => Some(
            v.split(',')
                .filter(|s| !s.trim().is_empty())
                .map(|s| parse_status(s.trim()))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        None => None,
    };
    let kind_filter: Option<Vec<TaskKind>> = match kind {
        Some(v) => Some(
            v.split(',')
                .filter(|s| !s.trim().is_empty())
                .map(|s| parse_kind(s.trim()))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        None => None,
    };
    let parent_filter = parent;

    let filtered: Vec<Task> = tasks
        .into_iter()
        .filter(|t| {
            if let Some(sf) = &status_filter
                && !sf.contains(&t.status)
            {
                return false;
            }
            if let Some(kf) = &kind_filter
                && !kf.contains(&t.kind)
            {
                return false;
            }
            if let Some(pf) = &parent_filter {
                if pf == "none" {
                    if t.parent.is_some() {
                        return false;
                    }
                } else if t.parent.as_deref() != Some(pf.as_str()) {
                    return false;
                }
            }
            true
        })
        .collect();

    if json {
        print_json(&filtered);
    } else {
        use std::io::Write;
        print!("{}", render_plan(&filtered));
        std::io::stdout().flush().ok();
    }
    Ok(0)
}

/// `task show <id>` — show task details + computed relationships.
pub fn task_show(store: &Store, id: &str, json: bool) -> Result<u8, String> {
    let all = store.load_all().map_err(|e| format!("store error: {e}"))?;
    let Some(task) = all.iter().find(|t| t.id == id) else {
        eprintln!("task not found: {id}");
        return Ok(1);
    };
    if json {
        let children = store::graph::children_of(&all, &task.id);
        let dependents = store::graph::dependents(&all, &task.id);
        print_json(&serde_json::json!({
            "task": task,
            "children": children,
            "depended_by": dependents,
        }));
    } else {
        print_task_detail(task, &all);
    }
    Ok(0)
}

/// `show <id>` — top-level alias for `task show <id>`.
pub fn show_cmd(id: &str, json: bool) -> Result<u8, String> {
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    let store = Store::new(&config.data_dir);
    task_show(&store, id, json)
}

/// Print full task detail with computed relationships.
pub fn print_task_detail(task: &Task, all: &[Task]) {
    let kind_str = match task.kind {
        TaskKind::Milestone => "milestone",
        TaskKind::Feature => "feature",
        TaskKind::Bug => "bug",
        TaskKind::Task => "task",
        TaskKind::Chore => "chore",
        TaskKind::Spike => "spike",
        TaskKind::Decision => "decision",
        TaskKind::Unknown => "unknown",
    };
    let status_str = match task.status {
        TaskStatus::Open => "open",
        TaskStatus::InProgress => "in_progress",
        TaskStatus::Failed => "failed",
        TaskStatus::Done => "done",
    };
    println!("id:          {}", task.id);
    println!("title:       {}", task.title);
    println!("kind:        {kind_str}");
    println!("status:      {status_str}");
    if task.attempts > 0 {
        println!(
            "attempts:    {} / {}",
            task.attempts,
            daemon::runner::MAX_ATTEMPTS
        );
    }
    match &task.parent {
        Some(p) => println!("parent:      {p}"),
        None => println!("parent:      -"),
    }
    if task.deps.is_empty() {
        println!("deps:        -");
    } else {
        println!("deps:        {}", task.deps.join(", "));
    }
    println!("description: {}", task.description);
    println!("acceptance:  {}", task.acceptance);
    println!("created_at:  {}", task.created_at);
    println!("updated_at:  {}", task.updated_at);

    let children = store::graph::children_of(all, &task.id);
    let dependents = store::graph::dependents(all, &task.id);

    if children.is_empty() {
        println!("children:    -");
    } else {
        println!("children:    {}", children.join(", "));
    }
    if dependents.is_empty() {
        println!("depended_by: -");
    } else {
        println!("depended_by: {}", dependents.join(", "));
    }
}

/// Parse a status string ("open"|"in_progress"|"failed"|"done") → TaskStatus.
pub fn parse_status(s: &str) -> Result<TaskStatus, String> {
    match s {
        "open" => Ok(TaskStatus::Open),
        "in_progress" => Ok(TaskStatus::InProgress),
        "failed" => Ok(TaskStatus::Failed),
        "done" => Ok(TaskStatus::Done),
        other => Err(format!(
            "invalid status `{other}` (expected: open|in_progress|failed|done)"
        )),
    }
}

/// Parse a kind string → TaskKind.
pub fn parse_kind(s: &str) -> Result<TaskKind, String> {
    match s {
        "milestone" => Ok(TaskKind::Milestone),
        "feature" => Ok(TaskKind::Feature),
        "bug" => Ok(TaskKind::Bug),
        "task" => Ok(TaskKind::Task),
        "chore" => Ok(TaskKind::Chore),
        "spike" => Ok(TaskKind::Spike),
        "decision" => Ok(TaskKind::Decision),
        other => Err(format!(
            "invalid kind `{other}` (expected: milestone|feature|bug|task|chore|spike|decision)"
        )),
    }
}

/// `plan` subcommand: render the task tree (+ optional Graphviz DOT).
pub fn plan_cmd(dot: bool, json: bool) -> Result<u8, String> {
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    let store = Store::new(&config.data_dir);
    let tasks = store.load_all().map_err(|e| format!("store error: {e}"))?;
    if json {
        print_json(&tasks);
    } else {
        use std::io::Write;
        if dot {
            print!("{}", render_dot(&tasks));
        } else {
            print!("{}", render_plan(&tasks));
        }
        std::io::stdout().flush().ok();
    }
    Ok(0)
}

/// `run` subcommand: spawn a worker for a task and return its outcome.
///
/// Resume/retry semantics (#47): an InProgress task with a live runner is
/// refused (abort it first); an InProgress task without one is an orphan
/// and gets resumed; a Failed task is retried until the attempt budget
/// (`daemon::runner::MAX_ATTEMPTS`) is exhausted.
pub fn run_cmd(id: &str) -> Result<u8, String> {
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    let store = Store::new(&config.data_dir);
    let Some(task) = store
        .load_task(id)
        .map_err(|e| format!("store error: {e}"))?
    else {
        eprintln!("task not found: {id}");
        return Ok(1);
    };

    // #47: live-runner guard — never double-spawn a worker for one task.
    let task_dir = config.data_dir.join("tasks").join(id);
    if task.status == TaskStatus::InProgress {
        if daemon::runner::runner_alive(&task_dir) {
            eprintln!(
                "task already running: {id} (live runner, see {}); use `cli abort {id}` first",
                task_dir.display()
            );
            return Ok(1);
        }
        eprintln!("resuming orphaned task: {id} (in_progress without a live runner)");
    } else if task.status == TaskStatus::Failed {
        eprintln!(
            "retrying failed task: {id} (attempt {}/{})",
            task.attempts + 1,
            daemon::runner::MAX_ATTEMPTS
        );
    }

    // Deps gate: refuse blocked task without spawning a worker.
    if !store::graph::is_ready(
        &store
            .load_all()
            .map_err(|e| format!("store error: {e}"))?
            .into_iter()
            .map(|t| (t.id.clone(), t))
            .collect(),
        id,
    ) {
        eprintln!("blocked: {id} -- predecessors not complete");
        return Ok(1);
    }

    // F3: mark the task running (InProgress) before the worker starts so the
    // board shows it live; the runner outcome flips it to Done/Failed.
    let mut running = task.clone();
    running.status = TaskStatus::InProgress;
    running.updated_at = store::now_iso();
    store
        .append(&running)
        .map_err(|e| format!("store error: {e}"))?;

    // Runner takes over from here; its outcome decides the store flip.
    let outcome = match daemon::runner::run(
        &daemon::runner::Ctx {
            omp_path: config.omp_path.clone(),
            data_dir: config.data_dir.clone(),
            mcp_servers: config.mcp_servers.clone(),
            tasks: store
                .load_all()
                .map_err(|e| format!("store error: {e}"))?
                .into_iter()
                .map(|t| (t.id.clone(), t))
                .collect(),
        },
        id,
    ) {
        Ok(outcome) => outcome,
        Err(e) => {
            // #47: pre-run rejection — roll back the InProgress marker so a
            // task that never actually ran is never stranded as in_progress.
            store
                .append(&task)
                .map_err(|se| format!("store error: {se}"))?;
            return Err(format!("runner error: {e}"));
        }
    };

    // #47: flip status, bump the failure counter, and record the attempt as
    // traceable evidence (attempts.jsonl).
    let updated = persist_run_outcome(&store, &config.data_dir, &task, &outcome)?;

    // Evidence drop per MVP §3.2: result.json in <data_dir>/tasks/<id>/.
    if let Err(e) = std::fs::create_dir_all(&task_dir) {
        eprintln!("warning: could not create task context dir: {e}");
    } else {
        let result_payload = serde_json::json!({
            "status": match outcome.status {
                daemon::runner::RunStatus::Done => "done",
                daemon::runner::RunStatus::Failed => "failed",
            },
            "summary": outcome.summary,
            "events_seen": outcome.events_seen,
            "finished_at": updated.updated_at,
        });
        let result_path = task_dir.join("result.json");
        if let Err(e) = std::fs::write(
            &result_path,
            serde_json::to_string_pretty(&result_payload).unwrap(),
        ) {
            eprintln!("warning: could not write result.json: {e}");
        }
    }

    if outcome.status == daemon::runner::RunStatus::Done {
        println!("done {id}");
        Ok(0)
    } else {
        eprintln!("run failed: {}", outcome.summary);
        if updated.attempts >= daemon::runner::MAX_ATTEMPTS {
            eprintln!(
                "retry limit reached ({} failed attempts); reset with `cli task update {id} --attempts 0`",
                updated.attempts
            );
        } else {
            eprintln!(
                "failed attempts: {}/{}; retry with `cli run {id}`",
                updated.attempts,
                daemon::runner::MAX_ATTEMPTS
            );
        }
        Ok(1)
    }
}

/// Persist one finished run attempt (#47): flip status to Done/Failed,
/// bump `attempts` on failure, and append the attempt (number, outcome,
/// reason, timestamp) to `<task_dir>/attempts.jsonl`.
pub fn persist_run_outcome(
    store: &Store,
    data_dir: &std::path::Path,
    pre: &Task,
    outcome: &daemon::runner::RunOutcome,
) -> Result<Task, String> {
    let attempt = pre.attempts + 1;
    let mut updated = pre.clone();
    updated.status = match outcome.status {
        daemon::runner::RunStatus::Done => TaskStatus::Done,
        daemon::runner::RunStatus::Failed => {
            updated.attempts = attempt;
            TaskStatus::Failed
        }
    };
    updated.updated_at = store::now_iso();
    record_attempt(data_dir, &pre.id, attempt, outcome);
    store
        .append(&updated)
        .map_err(|e| format!("store error: {e}"))?;
    Ok(updated)
}

/// Append one attempt record to `attempts.jsonl` (#47 evidence).
/// Best-effort like the event log: a failing evidence write warns on stderr
/// and never breaks the run or the store flip.
pub fn record_attempt(
    data_dir: &std::path::Path,
    task_id: &str,
    attempt: u32,
    outcome: &daemon::runner::RunOutcome,
) {
    let task_dir = data_dir.join("tasks").join(task_id);
    if let Err(e) = std::fs::create_dir_all(&task_dir) {
        eprintln!("warning: could not create task context dir: {e}");
        return;
    }
    let rec = serde_json::json!({
        "ts": store::now_iso(),
        "attempt": attempt,
        "outcome": match outcome.status {
            daemon::runner::RunStatus::Done => "done",
            daemon::runner::RunStatus::Failed => "failed",
        },
        "reason": outcome.summary,
        "events_seen": outcome.events_seen,
    });
    let path = task_dir.join("attempts.jsonl");
    let res = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| {
            use std::io::Write;
            writeln!(f, "{}", serde_json::to_string(&rec).unwrap_or_default())
        });
    if let Err(e) = res {
        eprintln!("warning: write {}: {e}", path.display());
    }
}

/// `steer` subcommand: if the task is running (InProgress), queue a live
/// steering message via `steer-cmd.txt` — the runner polls and forwards it
/// to the worker. Non-running tasks get a stored note instead.
pub fn steer_cmd(id: &str, message: &[String], json: bool) -> Result<u8, String> {
    if message.is_empty() {
        return Err(format!("usage: cli steer {id} <message>"));
    }
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    let store = Store::new(&config.data_dir);
    let Some(task) = store
        .load_task(id)
        .map_err(|e| format!("store error: {e}"))?
    else {
        eprintln!("task not found: {id}");
        return Ok(1);
    };
    let msg = message.join(" ");
    if task.status == TaskStatus::InProgress {
        let dir = config.data_dir.join("tasks").join(id);
        std::fs::create_dir_all(&dir).map_err(|e| format!("create task dir: {e}"))?;
        std::fs::write(dir.join("steer-cmd.txt"), &msg)
            .map_err(|e| format!("write steer inbox: {e}"))?;
        let note = format!("steer queued for running task {id}: {msg}");
        if json {
            json_ok(&note);
        } else {
            println!("{note}");
        }
    } else {
        let note = format!(
            "steer note for {id}: {msg} (task not running; status {:?})",
            task.status
        );
        if json {
            json_ok(&note);
        } else {
            println!("{note}");
        }
    }
    Ok(0)
}

/// `abort` subcommand: if the task is running (InProgress), SIGTERM both the
/// omp worker's process group (`kill -TERM -<omp-pid>`, it is a group leader
/// via `process_group(0)`) and the runner process, then reset the task to
/// open so it can be re-run. Signal (not a polled file) because the worker
/// may be mid-thinking with no events to poll between. Non-running tasks are
/// reset to open directly.
pub fn abort_cmd(id: &str, json: bool) -> Result<u8, String> {
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    let store = Store::new(&config.data_dir);
    let Some(mut task) = store
        .load_task(id)
        .map_err(|e| format!("store error: {e}"))?
    else {
        eprintln!("task not found: {id}");
        return Ok(1);
    };
    let msg;
    if task.status == TaskStatus::InProgress {
        let dir = config.data_dir.join("tasks").join(id);
        let pids: Vec<i32> = std::fs::read_to_string(dir.join("run.pid"))
            .ok()
            .map(|s| {
                s.split_whitespace()
                    .filter_map(|p| p.parse::<i32>().ok())
                    .collect()
            })
            .unwrap_or_default();
        if pids.len() == 2 {
            let (run_pid, omp_pid) = (pids[0], pids[1]);
            // Worker tree first (process group), then the runner itself.
            // .output() discards kill's stderr — abort success is defined by
            // the store reset below, not by kill diagnostics.
            let _ = std::process::Command::new("kill")
                .args(["-TERM", &format!("-{omp_pid}")])
                .output();
            let _ = std::process::Command::new("kill")
                .args(["-TERM", &run_pid.to_string()])
                .output();
            msg = format!("abort sent to omp group {omp_pid} + runner {run_pid} for {id}");
        } else {
            msg = format!("no live run for {id}; reset to open");
        }
        task.status = TaskStatus::Open;
        task.updated_at = store::now_iso();
        store
            .append(&task)
            .map_err(|e| format!("store error: {e}"))?;
    } else {
        msg = format!("aborted {id}; status reset to open");
    }
    if json {
        json_ok(&msg);
    } else {
        println!("{msg}");
    }
    Ok(0)
}

/// `compact` subcommand: compact the store (latest-per-id, drop tombstones).
pub fn compact_cmd(json: bool) -> Result<u8, String> {
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    let store = Store::new(&config.data_dir);
    let before = store
        .load_all()
        .map_err(|e| format!("store error: {e}"))?
        .len();
    store.compact().map_err(|e| format!("store error: {e}"))?;
    let after = store
        .load_all()
        .map_err(|e| format!("store error: {e}"))?
        .len();
    let msg = format!("compacted: {before} -> {after} tasks");
    if json {
        json_ok(&msg);
    } else {
        println!("{msg}");
    }
    Ok(0)
}

/// `init` subcommand: create `.oi/` (config dir), `.oi/config.toml` and the
/// spec template files under `.oi/specs/`. Idempotent — existing artifacts
/// are never overwritten.
pub fn init_cmd(json: bool) -> Result<u8, String> {
    let dir = std::env::current_dir().map_err(|e| format!("cwd error: {e}"))?;
    init_cmd_at(&dir, json)
}

/// init internals, testable with an explicit directory.
pub fn init_cmd_at(dir: &std::path::Path, json: bool) -> Result<u8, String> {
    let oi_dir = dir.join(".oi");
    let config_path = oi_dir.join("config.toml");
    let dir_existed = oi_dir.exists();
    let config_existed = config_path.exists();

    if !dir_existed {
        std::fs::create_dir_all(&oi_dir).map_err(|e| format!("could not create .oi/: {e}"))?;
    }
    if !config_existed {
        let default = "omp_path = \"omp\"\n\
                       data_dir = \"./.oi\"\n\
                       model = \"default\"\n";
        std::fs::write(&config_path, default)
            .map_err(|e| format!("could not create .oi/config.toml: {e}"))?;
    }
    // Spec templates (never overwrite user edits).
    spec::template::init::write_default_specs(&oi_dir)
        .map_err(|e| format!("spec templates: {e}"))?;
    // Task templates (never overwrite user edits).
    store::template::write_default_templates(&oi_dir)
        .map_err(|e| format!("task templates: {e}"))?;
    let msg = match (dir_existed, config_existed) {
        (true, true) => "workspace already initialized",
        (false, false) => "initialized: .oi/, .oi/config.toml, .oi/specs/",
        (false, true) => "created: .oi/, .oi/specs/",
        (true, false) => "created: .oi/config.toml, .oi/specs/",
    };
    if json {
        json_ok(msg);
    } else {
        println!("{msg}");
    }
    Ok(0)
}

/// Embedded boot/bundle profiles. Files live at the repo root `profiles/`;
/// embedded (not read from disk) so the binary works from any cwd.
const PROFILES: &[(&str, &str)] = &[
    ("boot", include_str!("../../../../../profiles/boot.toml")),
    (
        "bundle",
        include_str!("../../../../../profiles/bundle.toml"),
    ),
];

/// `profile list` -- 名字 + 文件首行描述。
pub fn profile_list_cmd(json: bool) -> Result<u8, String> {
    let rows: Vec<(&str, &str)> = PROFILES
        .iter()
        .map(|(name, body)| (*name, profile_blurb(body)))
        .collect();
    if json {
        let value: Vec<serde_json::Value> = rows
            .iter()
            .map(|(name, blurb)| serde_json::json!({ "name": name, "description": blurb }))
            .collect();
        print_json(&value);
    } else {
        for (name, blurb) in &rows {
            println!("{name}\t{blurb}");
        }
    }
    Ok(0)
}

/// 文件首条 `#` 注释即描述（ profiles 的约定：第一行写用途）。
pub fn profile_blurb(body: &str) -> &str {
    body.lines()
        .find_map(|l| l.strip_prefix("# "))
        .unwrap_or("")
        .trim()
}

/// `profile apply <name>` -- 把 profile 写进 `<dir>/.oi/config.toml`。
/// 与 `init` 同一语义：已存在则拒绝覆盖（返回非零），不动用户配置。
pub fn profile_apply_cmd_at(dir: &std::path::Path, name: &str, json: bool) -> Result<u8, String> {
    let Some((_, body)) = PROFILES.iter().find(|(n, _)| *n == name) else {
        return Err(format!(
            "unknown profile '{name}' (available: {})",
            PROFILES
                .iter()
                .map(|(n, _)| *n)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    };
    let oi_dir = dir.join(".oi");
    let config_path = oi_dir.join("config.toml");
    if config_path.exists() {
        return Err(format!(
            "{} already exists; refusing to overwrite",
            config_path.display()
        ));
    }
    std::fs::create_dir_all(&oi_dir).map_err(|e| format!("could not create .oi/: {e}"))?;
    std::fs::write(&config_path, body)
        .map_err(|e| format!("could not write .oi/config.toml: {e}"))?;
    let msg = format!("applied profile '{name}': {}", config_path.display());
    if json {
        json_ok(&msg);
    } else {
        println!("{msg}");
    }
    Ok(0)
}

/// `ready` -- list open tasks whose deps are all done, sorted by priority then id.
pub fn ready_cmd(json: bool) -> Result<u8, String> {
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    let store = Store::new(&config.data_dir);
    let all = store.load_all().map_err(|e| format!("store error: {e}"))?;
    let map: std::collections::HashMap<String, Task> =
        all.iter().map(|t| (t.id.clone(), t.clone())).collect();

    let mut ready: Vec<&Task> = all
        .iter()
        .filter(|t| t.status == TaskStatus::Open && store::graph::is_ready(&map, &t.id))
        .collect();
    ready.sort_by(|a, b| a.priority.cmp(&b.priority).then(a.id.cmp(&b.id)));

    if json {
        let ids: Vec<&str> = ready.iter().map(|t| t.id.as_str()).collect();
        print_json(&ids);
    } else if ready.is_empty() {
        println!("(no ready tasks)");
    } else {
        for t in &ready {
            println!("\u{25cb} {} P{} {}", t.id, t.priority, t.title);
        }
    }
    Ok(0)
}

/// `blocked` -- list tasks with unmet deps, sorted by priority then id.
pub fn blocked_cmd(json: bool) -> Result<u8, String> {
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    let store = Store::new(&config.data_dir);
    let all = store.load_all().map_err(|e| format!("store error: {e}"))?;
    let map: std::collections::HashMap<String, Task> =
        all.iter().map(|t| (t.id.clone(), t.clone())).collect();

    let mut blocked: Vec<(&Task, Vec<String>)> = all
        .iter()
        .filter_map(|t| {
            if t.status == TaskStatus::Done {
                return None;
            }
            let unmet: Vec<String> = t
                .deps
                .iter()
                .filter(|dep| map.get(*dep).is_none_or(|d| d.status != TaskStatus::Done))
                .cloned()
                .collect();
            if unmet.is_empty() {
                None
            } else {
                Some((t, unmet))
            }
        })
        .collect();
    blocked.sort_by(|(a, _), (b, _)| a.priority.cmp(&b.priority).then(a.id.cmp(&b.id)));

    if json {
        let entries: Vec<serde_json::Value> = blocked
            .iter()
            .map(|(t, unmet)| serde_json::json!({"id": t.id, "blocked_by": unmet}))
            .collect();
        print_json(&entries);
    } else if blocked.is_empty() {
        println!("(no blocked tasks)");
    } else {
        for (t, unmet) in &blocked {
            println!(
                "\u{25cf} {} P{} {} -- blocked by: {}",
                t.id,
                t.priority,
                t.title,
                unmet.join(", ")
            );
        }
    }
    Ok(0)
}

/// cx/bd-style status glyph: `○` open-ready, `●` open-blocked,
/// `◐` in_progress, `✗` failed, `✓` done.
pub fn status_glyph(t: &Task, map: &std::collections::HashMap<String, Task>) -> &'static str {
    match t.status {
        TaskStatus::Done => "✓",
        TaskStatus::InProgress => "◐",
        TaskStatus::Failed => "✗",
        TaskStatus::Open => {
            if store::graph::is_ready(map, &t.id) {
                "○"
            } else {
                "●"
            }
        }
    }
}

/// cx task_line format: `<icon> <id> ● P<priority> <title>`.
pub fn task_line(t: &Task, map: &std::collections::HashMap<String, Task>) -> String {
    format!(
        "{} {} ● P{} {}",
        status_glyph(t, map),
        t.id,
        t.priority,
        t.title
    )
}

/// Status legend footer, matching bd/cx output conventions.
pub fn status_legend() -> &'static str {
    "Status: ○ open  ◐ in_progress  ✗ failed  ● blocked  ✓ done\n"
}

/// Render the task tree as an indented plan view (roots first, children
/// nested under their parent with box-drawing prefixes), cx/bd style.
///
/// Tasks whose `parent` is missing (dangling) or `None` are treated as roots.
/// A visited set guards against parent cycles in malformed stores.
pub fn render_plan(tasks: &[Task]) -> String {
    use std::collections::{HashMap, HashSet};

    if tasks.is_empty() {
        return "(no tasks)\n".to_string();
    }

    let map: HashMap<String, Task> = tasks.iter().map(|t| (t.id.clone(), t.clone())).collect();
    let ids: HashSet<&str> = tasks.iter().map(|t| t.id.as_str()).collect();

    let mut children: HashMap<&str, Vec<&Task>> = HashMap::new();
    let mut roots: Vec<&Task> = Vec::new();
    for t in tasks {
        match t.parent.as_deref() {
            Some(p) if ids.contains(p) => children.entry(p).or_default().push(t),
            _ => roots.push(t),
        }
    }

    /// Stable topological order of a sibling list: if `a` depends on `b`
    /// (both in the list), `b` renders before `a`. Kahn's algorithm with
    /// declaration order as the tie-breaker (sibling chain from #216).
    fn topo_sort<'a>(kids: &[&'a Task], map: &HashMap<String, Task>) -> Vec<&'a Task> {
        use std::collections::HashMap as H;
        let ids: std::collections::HashSet<&str> = kids.iter().map(|k| k.id.as_str()).collect();
        let mut indeg: H<&str, usize> = H::new();
        let mut deps_map: H<&str, Vec<&str>> = H::new();
        let mut order: H<&str, usize> = H::new();
        for (i, k) in kids.iter().enumerate() {
            indeg.insert(k.id.as_str(), 0);
            order.insert(k.id.as_str(), i);
        }
        for k in kids {
            for d in &k.deps {
                if ids.contains(d.as_str()) && d != &k.id {
                    deps_map.entry(d.as_str()).or_default().push(k.id.as_str());
                    *indeg.get_mut(k.id.as_str()).unwrap() += 1;
                }
            }
        }
        let mut queue: Vec<&Task> = kids
            .iter()
            .filter(|k| indeg[k.id.as_str()] == 0)
            .copied()
            .collect();
        let mut out = Vec::new();
        while !queue.is_empty() {
            queue.sort_by_key(|k| order[k.id.as_str()]);
            let n = queue.remove(0);
            out.push(n);
            if let Some(ms) = deps_map.get(n.id.as_str()).cloned() {
                for m in ms {
                    let e = indeg.get_mut(m).unwrap();
                    *e -= 1;
                    if *e == 0
                        && let Some(t) = kids.iter().find(|k| k.id == m)
                    {
                        queue.push(t);
                    }
                }
            }
        }
        let _ = map; // reserved: ready/blocked glyph already computed by task_line
        out
    }

    fn print_children(
        parent: &Task,
        children: &HashMap<&str, Vec<&Task>>,
        map: &HashMap<String, Task>,
        prefix: &str,
        visited: &mut HashSet<String>,
        out: &mut String,
    ) {
        let Some(kids) = children.get(parent.id.as_str()) else {
            return;
        };
        for (i, kid) in topo_sort(kids, map).iter().enumerate() {
            let is_last = i == kids.len() - 1;
            let branch = if is_last { "└─ " } else { "├─ " };
            // Mark before printing so a cycle back-edge is skipped, not re-printed.
            if !visited.insert(kid.id.clone()) {
                continue;
            }
            out.push_str(&format!("{prefix}{branch}{}\n", task_line(kid, map)));
            let next_prefix = format!("{prefix}{}", if is_last { "   " } else { "│  " });
            print_children(kid, children, map, &next_prefix, visited, out);
        }
    }

    let mut visited: HashSet<String> = HashSet::new();
    let mut out = String::new();
    // Root tasks first; a visited guard prevents cycles from re-printing.
    for root in &roots {
        if !visited.insert(root.id.clone()) {
            continue;
        }
        out.push_str(&format!("{}\n", task_line(root, &map)));
        print_children(root, &children, &map, "", &mut visited, &mut out);
    }
    // Fallback: tasks in a pure parent-cycle (no root exists) still show once.
    for t in tasks {
        if !visited.insert(t.id.clone()) {
            continue;
        }
        out.push_str(&format!("{}\n", task_line(t, &map)));
        print_children(t, &children, &map, "", &mut visited, &mut out);
    }
    out.push_str(status_legend());
    out
}

/// Return ids of tasks newly unblocked by completing `done_id`: status Open,
/// all deps Done (via `is_ready`), and `done_id` listed among their deps.
pub fn suggest_next(tasks: &[Task], done_id: &str) -> Vec<String> {
    use std::collections::HashMap;
    let map: HashMap<String, Task> = tasks.iter().map(|t| (t.id.clone(), t.clone())).collect();
    tasks
        .iter()
        .filter(|t| {
            t.status == TaskStatus::Open
                && t.deps.iter().any(|d| d == done_id)
                && store::graph::is_ready(&map, &t.id)
        })
        .map(|t| t.id.clone())
        .collect()
}

/// Render the task graph as Graphviz DOT.
///
/// Dependency edges are solid (`dep -> task`); parent→child edges are dotted
/// with `arrowhead=none`. Nodes are colored by status.
pub fn render_dot(tasks: &[Task]) -> String {
    if tasks.is_empty() {
        return "digraph omenic {\n}\n".to_string();
    }

    fn status_color(s: &TaskStatus) -> &'static str {
        match s {
            TaskStatus::Open => "#e8f4fd",
            TaskStatus::InProgress => "#fff3cd",
            TaskStatus::Failed => "#f8d7da",
            TaskStatus::Done => "#d4edda",
        }
    }

    let mut out = String::new();
    out.push_str("digraph omenic {\n");
    out.push_str("  rankdir=LR;\n");
    out.push_str("  node [shape=box, style=\"rounded,filled\"];\n");

    // Nodes
    for t in tasks {
        let color = status_color(&t.status);
        let esc_id = t.id.replace('\\', "\\\\").replace('"', "\\\"");
        let esc_title = t.title.replace('\\', "\\\\").replace('"', "\\\"");
        out.push_str(&format!(
            "  \"{esc_id}\" [label=\"{esc_id}\\nP{} | {esc_title}\", fillcolor=\"{color}\"];\n",
            t.priority
        ));
    }

    // Dependency edges (solid): dep -> task
    for t in tasks {
        let esc_id = t.id.replace('\\', "\\\\").replace('"', "\\\"");
        for dep in &t.deps {
            let esc_dep = dep.replace('\\', "\\\\").replace('"', "\\\"");
            out.push_str(&format!("  \"{esc_dep}\" -> \"{esc_id}\";\n"));
        }
    }

    // Parent → child edges (dotted, no arrowhead)
    for t in tasks {
        let esc_id = t.id.replace('\\', "\\\\").replace('"', "\\\"");
        if let Some(parent) = &t.parent {
            let esc_parent = parent.replace('\\', "\\\\").replace('"', "\\\"");
            out.push_str(&format!(
                "  \"{esc_parent}\" -> \"{esc_id}\" [style=dotted, arrowhead=none];\n"
            ));
        }
    }

    out.push_str("}\n");
    out
}

/// `board` subcommand: the task running board — tasks partitioned by
/// status/readiness so an agent can see what to run next and what is blocked.
pub fn board_cmd(json: bool) -> Result<u8, String> {
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    let store = Store::new(&config.data_dir);
    let all = store.load_all().map_err(|e| format!("store error: {e}"))?;
    let map: std::collections::HashMap<String, Task> =
        all.iter().map(|t| (t.id.clone(), t.clone())).collect();

    let mut done: Vec<&Task> = Vec::new();
    let mut in_progress: Vec<&Task> = Vec::new();
    let mut failed: Vec<&Task> = Vec::new();
    let mut ready: Vec<&Task> = Vec::new();
    let mut blocked: Vec<&Task> = Vec::new();
    for t in &all {
        match t.status {
            TaskStatus::Done => done.push(t),
            TaskStatus::InProgress => in_progress.push(t),
            TaskStatus::Failed => failed.push(t),
            TaskStatus::Open => {
                if store::graph::is_ready(&map, &t.id) {
                    ready.push(t);
                } else {
                    blocked.push(t);
                }
            }
        }
    }
    let by_priority = |a: &&Task, b: &&Task| a.priority.cmp(&b.priority).then(a.id.cmp(&b.id));
    done.sort_by(by_priority);
    in_progress.sort_by(by_priority);
    ready.sort_by(by_priority);
    blocked.sort_by(by_priority);
    failed.sort_by(by_priority);

    if json {
        let part = |v: &[&Task]| -> Vec<serde_json::Value> {
            v.iter()
                .map(|t| {
                    serde_json::json!({
                        "id": t.id,
                        "title": t.title,
                        "priority": t.priority,
                        "deps": t.deps,
                    })
                })
                .collect()
        };
        let obj = serde_json::json!({
            "done": part(&done),
            "in_progress": part(&in_progress),
            "failed": part(&failed),
            "ready": part(&ready),
            "blocked": part(&blocked),
        });
        print_json(&obj);
    } else {
        let section = |name: &str, v: &[&Task]| -> String {
            let mut s = format!("## {name} ({})\n", v.len());
            for t in v {
                s.push_str(&format!("  {}\n", task_line(t, &map)));
            }
            s
        };
        print!(
            "{}{}{}{}{}{}",
            section("done", &done),
            section("in_progress", &in_progress),
            section("failed", &failed),
            section("ready", &ready),
            section("blocked", &blocked),
            status_legend(),
        );
    }
    Ok(0)
}

/// `template list` subcommand: list orchestration templates from
/// `<data>/templates/{phases,steps}/`.
pub fn template_list_cmd(json: bool) -> Result<u8, String> {
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    let templates = store::template::load_all_templates(&config.data_dir)
        .map_err(|e| format!("template error: {e}"))?;
    if templates.is_empty() {
        eprintln!(
            "no templates in {} (run `cli init` first)",
            config.data_dir.join("templates").display()
        );
        return Ok(1);
    }
    if json {
        let list: Vec<serde_json::Value> = templates
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "kind": match t.kind {
                        store::template::TemplateKind::Phase => "phase",
                        store::template::TemplateKind::Step => "step",
                    },
                    "tasks": t.tasks.iter().map(|x| x.key.clone()).collect::<Vec<_>>(),
                })
            })
            .collect();
        print_json(&list);
    } else {
        for t in &templates {
            let kind = match t.kind {
                store::template::TemplateKind::Phase => "phase",
                store::template::TemplateKind::Step => "step",
            };
            println!(
                "{kind}: {} — {}",
                t.name,
                t.tasks.first().map(|x| x.title.as_str()).unwrap_or("")
            );
            for x in &t.tasks {
                println!("  - {}", x.key);
            }
        }
    }
    Ok(0)
}

/// `template apply` subcommand: create topic + phase + steps from a template.
pub fn template_apply_cmd(
    store: &Store,
    name: &str,
    topic: &str,
    parent: Option<String>,
    json: bool,
) -> Result<u8, String> {
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    let ids = store::template::apply(store, &config.data_dir, name, topic, parent)
        .map_err(|e| format!("template error: {e} — try `cli template list` or `cli init`"))?;
    if json {
        let obj = serde_json::json!({ "created": ids });
        print_json(&obj);
    } else {
        for id in &ids {
            println!("created {id}");
        }
    }
    Ok(0)
}

/// `spec list` subcommand: list spec tables loaded from `<data>/specs/`.
pub fn spec_list_cmd(json: bool) -> Result<u8, String> {
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    let specs = spec::template::parse::load_all_specs(&config.data_dir)
        .map_err(|e| format!("spec error: {e}"))?;
    if specs.is_empty() {
        eprintln!(
            "no spec templates in {} (run `cli init` first)",
            config.data_dir.join("specs").display()
        );
        return Ok(1);
    }
    if json {
        let list: Vec<serde_json::Value> = specs
            .iter()
            .map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "description": s.description,
                    "fields": s.fields.iter().map(|f| {
                        serde_json::json!({
                            "heading": f.heading,
                            "required": f.required,
                            "checkbox": f.checkbox,
                        })
                    }).collect::<Vec<_>>(),
                })
            })
            .collect();
        print_json(&list);
    } else {
        for s in &specs {
            println!("{} — {}", s.name, s.description);
            for f in &s.fields {
                let mark = if f.required { "req" } else { "opt" };
                let cb = if f.checkbox { " [checkbox]" } else { "" };
                println!("  {mark}: ## {}{cb}", f.heading);
            }
        }
    }
    Ok(0)
}

/// `spec new` subcommand: generate a blank spec table skeleton.
pub fn spec_new_cmd(
    kind: &str,
    title: Option<String>,
    output: Option<String>,
    json: bool,
) -> Result<u8, String> {
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    let spec = spec::template::parse::load_spec(&config.data_dir, kind)
        .map_err(|e| format!("spec error: {e} — try `cli spec list` or `cli init`"))?;
    let doc = spec::template::render::render_skeleton(&spec, title.as_deref().unwrap_or(""));
    match output {
        Some(path) => {
            std::fs::write(&path, &doc).map_err(|e| format!("write {}: {e}", path))?;
            let msg = format!("spec skeleton written to {path}");
            if json {
                json_ok(&msg);
            } else {
                println!("{msg}");
            }
        }
        None => {
            use std::io::Write;
            print!("{doc}");
            std::io::stdout().flush().ok();
        }
    }
    Ok(0)
}

/// `spec check` subcommand: validate a filled document against the kind's rules.
pub fn spec_check_cmd(kind: &str, file: &str, json: bool) -> Result<u8, String> {
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    let spec = spec::template::parse::load_spec(&config.data_dir, kind)
        .map_err(|e| format!("spec error: {e} — try `cli spec list` or `cli init`"))?;
    let findings = spec::template::check::check_file(&spec, std::path::Path::new(file))?;
    let fails: Vec<_> = findings.iter().filter(|f| f.fail).collect();
    if json {
        let obj = serde_json::json!({
            "spec": spec.name,
            "file": file,
            "ok": fails.is_empty(),
            "findings": findings.iter().map(|f| serde_json::json!({
                "rule": f.rule,
                "fail": f.fail,
                "message": f.message,
            })).collect::<Vec<_>>(),
        });
        print_json(&obj);
    } else {
        for f in &findings {
            let tag = if f.fail { "FAIL" } else { "ok  " };
            println!("{tag} [{}] {}", f.rule, f.message);
        }
        if fails.is_empty() {
            println!("RESULT: ALL PASS");
        } else {
            println!("RESULT: FAIL ({} issue(s))", fails.len());
        }
    }
    Ok(if fails.is_empty() { 0 } else { 1 })
}

/// `spec view` subcommand: print a spec document for the agent to inspect.
pub fn spec_view_cmd(file: &str) -> Result<u8, String> {
    let doc = std::fs::read_to_string(file).map_err(|e| format!("read {}: {e}", file))?;
    use std::io::Write;
    print!("{doc}");
    std::io::stdout().flush().ok();
    Ok(0)
}

/// `pr render <task-id>` subcommand: render a task subtree (topic → phase →
/// steps, sibling order from deps) as a PR Construction plan checkbox list.
pub fn pr_render_cmd(id: &str, json: bool) -> Result<u8, String> {
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    let store = Store::new(&config.data_dir);
    let all = store.load_all().map_err(|e| format!("store error: {e}"))?;
    if !all.iter().any(|t| t.id == id) {
        eprintln!("task not found: {id}");
        return Ok(1);
    }
    let map: std::collections::HashMap<String, Task> =
        all.iter().map(|t| (t.id.clone(), t.clone())).collect();

    // Children of a parent, in dependency-topological order (Kahn).
    fn children_of<'a>(all: &'a [Task], parent: &str) -> Vec<&'a Task> {
        let kids: Vec<&Task> = all
            .iter()
            .filter(|t| t.parent.as_deref() == Some(parent))
            .collect();
        let ids: std::collections::HashSet<&str> = kids.iter().map(|k| k.id.as_str()).collect();
        let mut indeg: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        let mut deps_map: std::collections::HashMap<&str, Vec<&str>> =
            std::collections::HashMap::new();
        for k in &kids {
            indeg.insert(k.id.as_str(), 0);
        }
        for k in &kids {
            for d in &k.deps {
                if ids.contains(d.as_str()) && d != &k.id {
                    deps_map.entry(d.as_str()).or_default().push(k.id.as_str());
                    *indeg.get_mut(k.id.as_str()).unwrap() += 1;
                }
            }
        }
        let mut queue: Vec<&Task> = kids
            .iter()
            .filter(|k| indeg[k.id.as_str()] == 0)
            .copied()
            .collect();
        let mut order: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for (i, k) in kids.iter().enumerate() {
            order.insert(k.id.as_str(), i);
        }
        let mut out = Vec::new();
        while !queue.is_empty() {
            queue.sort_by_key(|k| order[k.id.as_str()]);
            let n = queue.remove(0);
            out.push(n);
            if let Some(ms) = deps_map.get(n.id.as_str()).cloned() {
                for m in ms {
                    let e = indeg.get_mut(m).unwrap();
                    *e -= 1;
                    if *e == 0
                        && let Some(t) = kids.iter().find(|k| k.id == m)
                    {
                        queue.push(t);
                    }
                }
            }
        }
        out
    }

    fn emit<'a>(
        t: &'a Task,
        all: &'a [Task],
        json_list: &mut Vec<serde_json::Value>,
        out: &mut String,
        depth: usize,
    ) {
        let pad = "  ".repeat(depth);
        out.push_str(&format!("{pad}- [ ] {}：{}\n", t.id, t.title));
        json_list.push(serde_json::json!({
            "id": t.id,
            "title": t.title,
            "depth": depth,
            "deps": t.deps,
        }));
        for kid in children_of(all, &t.id) {
            emit(kid, all, json_list, out, depth + 1);
        }
    }

    let root = &map[id];
    let mut out = String::from("## Construction plan\n");
    let mut json_list: Vec<serde_json::Value> = Vec::new();
    out.push_str(&format!("- [ ] {}：{}\n", root.id, root.title));
    json_list.push(
        serde_json::json!({"id": root.id, "title": root.title, "depth": 0, "deps": root.deps}),
    );
    for kid in children_of(&all, id) {
        emit(kid, &all, &mut json_list, &mut out, 1);
    }
    if json {
        print_json(&json_list);
    } else {
        use std::io::Write;
        print!("{out}");
        std::io::stdout().flush().ok();
    }
    Ok(0)
}
