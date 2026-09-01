//! CLI command layer.
//!
//! Subcommands: task add/done/status/update/delete/list/show, plan, run,
//! steer, abort, ready, blocked, compact, init, dep.
//! Parsing uses clap (derive API); `--json` selects machine-readable output.

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tools::Tool;

use config::Config;
use task::store::Store;
use task::{Task, TaskKind, TaskStatus};
// clap CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "oi", about = "Task-driven agent orchestrator")]
struct Cli {
    /// Output JSON instead of human-readable text
    #[arg(long, global = true)]
    json: bool,

    /// Run non-interactive TUI smoke test (no subcommand mode only)
    #[arg(long)]
    test: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Task management commands
    Task {
        #[command(subcommand)]
        sub: TaskCmd,
    },
    /// Show task details (alias for `task show`)
    Show { id: String },
    /// Render the task tree / Graphviz DOT / board view
    Plan {
        /// Output Graphviz DOT format
        #[arg(long)]
        dot: bool,
        #[command(subcommand)]
        sub: Option<PlanSub>,
    },
    /// Execute a task via a worker session
    Run { id: String },
    /// Send a steering message to a running task
    Steer { id: String, message: Vec<String> },
    /// Abort a running task and reopen it
    Abort { id: String },
    /// List tasks ready to work on
    Ready,
    /// List tasks blocked by unmet dependencies
    Blocked,
    /// Compact the store (latest-per-id, drop tombstones)
    Compact,
    /// Initialize an omenic workspace
    Init,
    /// Manage task dependencies
    Dep {
        #[command(subcommand)]
        sub: DepCmd,
    },
    /// Task board: tasks partitioned by status/readiness
    Board,
    /// Built-in orchestration templates
    Template {
        #[command(subcommand)]
        sub: TemplateCmd,
    },
    /// Spec tables (规范表) for GitHub artifacts
    Spec {
        #[command(subcommand)]
        sub: SpecCmd,
    },
    /// Render task tree as PR Construction plan
    Pr {
        #[command(subcommand)]
        sub: PrCmd,
    },
    /// Read-only parallel subagent exploration (opt-in)
    Subagent {
        #[command(subcommand)]
        sub: SubagentCmd,
    },
}

/// Sub-views of `cli subagent`.
#[derive(Subcommand)]
enum SubagentCmd {
    /// Run one or more read-only subagents in parallel
    Run {
        /// One or more prompts; each spawns its own subagent.
        #[arg(long = "prompt", value_name = "PROMPT")]
        prompts: Vec<String>,
        /// Per-subagent loop turn cap (default 10).
        #[arg(long, default_value_t = subagent::config::MAX_TURNS_DEFAULT as u32)]
        max_turns: u32,
    },
}

/// Sub-views of `cli pr`.
#[derive(Subcommand)]
enum PrCmd {
    /// Render a task subtree as Construction plan checkboxes
    Render { id: String },
}

/// Sub-views of `cli plan`.
#[derive(Subcommand)]
enum PlanSub {
    /// Partitioned board view (ready / blocked / in_progress / done)
    Board,
}

#[derive(Subcommand)]
enum TaskCmd {
    /// Create a task
    Add {
        /// Task title(s); multiple positional args create multiple tasks
        title: Vec<String>,
        /// Parent task id
        #[arg(short = 'p', long)]
        parent: Option<String>,
        /// Comma-separated dependency ids
        #[arg(long)]
        deps: Option<String>,
        /// Acceptance criteria text
        #[arg(long)]
        acceptance: Option<String>,
        /// Priority 0-4 (0 highest)
        #[arg(long)]
        priority: Option<u8>,
        /// Task kind (milestone|feature|bug|task|chore|spike|decision)
        #[arg(long)]
        kind: Option<String>,
    },
    /// Mark a task done
    Done { id: String },
    /// Show a task's state
    Status { id: String },
    /// Update task fields
    Update {
        id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        deps: Option<String>,
        #[arg(long)]
        acceptance: Option<String>,
        #[arg(long)]
        priority: Option<u8>,
        #[arg(long)]
        kind: Option<String>,
        /// Reset the failed-attempt counter (#47 retry budget)
        #[arg(long)]
        attempts: Option<u32>,
    },
    /// Delete an isolated task
    Delete { id: String },
    /// List tasks (optionally filtered)
    List {
        /// Filter by status (open|in_progress|failed|done, comma-separated)
        #[arg(long)]
        status: Option<String>,
        /// Filter by kind (comma-separated)
        #[arg(long)]
        kind: Option<String>,
        /// Filter by parent id (`none` for root tasks)
        #[arg(long)]
        parent: Option<String>,
    },
    /// Show task details + computed relationships
    Show { id: String },
}

#[derive(Subcommand)]
enum DepCmd {
    /// Add a dependency edge
    Add { id: String, dep_id: String },
    /// Remove a dependency edge
    Remove { id: String, dep_id: String },
}

#[derive(Subcommand)]
enum TemplateCmd {
    /// List built-in templates
    List,
    /// Apply a template: create topic task + ordered step chain
    Apply {
        /// Template name (dev | plan)
        name: String,
        /// Topic title; becomes the parent task id
        topic: String,
        /// Parent task id for the topic (e.g. a milestone)
        #[arg(short = 'p', long)]
        parent: Option<String>,
    },
}

#[derive(Subcommand)]
enum SpecCmd {
    /// List spec tables
    List,
    /// Generate a blank spec table skeleton
    New {
        /// Spec kind (issue | epic | pr | review)
        kind: String,
        /// Document title (becomes the `#` heading)
        #[arg(long)]
        title: Option<String>,
        /// Write to file instead of stdout
        #[arg(short = 'o', long)]
        output: Option<String>,
    },
    /// Validate a filled spec document against the kind's rules
    Check {
        /// Spec kind to validate against
        kind: String,
        /// Markdown file to check
        file: String,
    },
    /// Print a spec document (agent-facing view)
    View { file: String },
}

/// Entry point: parse argv via clap and dispatch to a subcommand.
pub fn run() -> ExitCode {
    let cli = Cli::parse();
    match dispatch(cli) {
        Ok(code) => ExitCode::from(code),
        Err(msg) => {
            eprintln!("omenic: {msg}");
            ExitCode::from(2)
        }
    }
}

fn dispatch(cli: Cli) -> Result<u8, String> {
    let json = cli.json;
    match cli.command {
        // No subcommand: launch interactive TUI (or smoke test).
        None => {
            let config = Config::load().map_err(|e| format!("config error: {e}"))?;
            if cli.test {
                tui::test_mode(&config)
                    .map(|_| 0)
                    .map_err(|e| format!("tui error: {e}"))
            } else {
                tui::run(&config)
                    .map(|_| 0)
                    .map_err(|e| format!("tui error: {e}"))
            }
        }
        Some(command) => dispatch_sub(command, json),
    }
}

/// Dispatch a subcommand to the matching implementation function.
fn dispatch_sub(command: Command, json: bool) -> Result<u8, String> {
    match command {
        Command::Task { sub } => {
            let config = Config::load().map_err(|e| format!("config error: {e}"))?;
            let store = Store::new(&config.data_dir);
            match sub {
                TaskCmd::Add {
                    title,
                    parent,
                    deps,
                    acceptance,
                    priority,
                    kind,
                } => task_add(
                    &store, &title, parent, deps, acceptance, priority, kind, json,
                ),
                TaskCmd::Done { id } => task_done(&store, &id, json),
                TaskCmd::Status { id } => task_status(&store, &id, json),
                TaskCmd::Update {
                    id,
                    title,
                    description,
                    status,
                    deps,
                    acceptance,
                    priority,
                    kind,
                    attempts,
                } => task_update(
                    &store,
                    &id,
                    title,
                    description,
                    status,
                    deps,
                    acceptance,
                    priority,
                    kind,
                    attempts,
                    json,
                ),
                TaskCmd::Delete { id } => task_delete(&store, &id, json),
                TaskCmd::List {
                    status,
                    kind,
                    parent,
                } => task_list(&store, status, kind, parent, json),
                TaskCmd::Show { id } => task_show(&store, &id, json),
            }
        }
        Command::Show { id } => show_cmd(&id, json),
        Command::Plan { dot, sub } => match sub {
            Some(PlanSub::Board) => board_cmd(json),
            None => plan_cmd(dot, json),
        },
        Command::Run { id } => run_cmd(&id),
        Command::Steer { id, message } => steer_cmd(&id, &message, json),
        Command::Abort { id } => abort_cmd(&id, json),
        Command::Ready => ready_cmd(json),
        Command::Blocked => blocked_cmd(json),
        Command::Compact => compact_cmd(json),
        Command::Init => init_cmd(json),
        Command::Dep { sub } => {
            let config = Config::load().map_err(|e| format!("config error: {e}"))?;
            let store = Store::new(&config.data_dir);
            match sub {
                DepCmd::Add { id, dep_id } => dep_add(&store, &id, &dep_id, json),
                DepCmd::Remove { id, dep_id } => dep_remove(&store, &id, &dep_id, json),
            }
        }
        Command::Board => board_cmd(json),
        Command::Template { sub } => {
            let config = Config::load().map_err(|e| format!("config error: {e}"))?;
            let store = Store::new(&config.data_dir);
            match sub {
                TemplateCmd::List => template_list_cmd(json),
                TemplateCmd::Apply {
                    name,
                    topic,
                    parent,
                } => template_apply_cmd(&store, &name, &topic, parent, json),
            }
        }
        Command::Spec { sub } => match sub {
            SpecCmd::List => spec_list_cmd(json),
            SpecCmd::New {
                kind,
                title,
                output,
            } => spec_new_cmd(&kind, title, output, json),
            SpecCmd::Check { kind, file } => spec_check_cmd(&kind, &file, json),
            SpecCmd::View { file } => spec_view_cmd(&file),
        },
        Command::Pr { sub } => match sub {
            PrCmd::Render { id } => pr_render_cmd(&id, json),
        },
        Command::Subagent { sub } => match sub {
            SubagentCmd::Run { prompts, max_turns } => {
                subagent_run_cmd(&prompts, max_turns as usize)
            }
        },
    }
}

/// `subagent run` — drive one or more prompts through the `TaskTool`. Single
/// prompt runs inline; multiple prompts run in parallel via std::thread::scope
/// inside the tool, with a 5-min wall clock cap. Result is printed to stdout.
fn subagent_run_cmd(prompts: &[String], max_turns: usize) -> Result<u8, String> {
    use std::sync::atomic::AtomicBool;
    if prompts.is_empty() {
        return Err("at least one --prompt is required".into());
    }
    let args = serde_json::json!({
        "prompts": prompts,
        "max_turns": max_turns,
    });
    let tool = subagent::TaskTool;
    let out = tool
        .execute(&args, &AtomicBool::new(false))
        .map_err(|e| format!("subagent: {e}"))?;
    print!("{out}");
    Ok(0)
}
/// Print a JSON value to stdout.
fn print_json<T: ?Sized + serde::Serialize>(value: &T) {
    println!(
        "{}",
        serde_json::to_string_pretty(value)
            .unwrap_or_else(|e| { format!("{{\"error\":\"json serialization failed: {e}\"}}") })
    );
}

/// Print `{"status":"ok","message":"..."}` when --json is set.
fn json_ok(message: &str) {
    let obj = serde_json::json!({"status": "ok", "message": message});
    print_json(&obj);
}

#[allow(clippy::too_many_arguments)]
fn task_add(
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
                if task::graph::would_dep_cycle(&sim, title, dep) {
                    return Err(format!(
                        "adding dependency `{title}` -> `{dep}` would create a cycle"
                    ));
                }
            }
        }
    }

    let mut created_ids = Vec::new();
    for title in titles {
        let now = task::now_iso();
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

fn task_done(store: &Store, id: &str, json: bool) -> Result<u8, String> {
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
    task.updated_at = task::now_iso();
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

fn task_status(store: &Store, id: &str, json: bool) -> Result<u8, String> {
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
fn dep_add(store: &Store, task_id: &str, dep_id: &str, json: bool) -> Result<u8, String> {
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
    if task::graph::would_dep_cycle(&sim, task_id, dep_id) {
        return Err(format!(
            "adding dependency `{task_id}` -> `{dep_id}` would create a cycle"
        ));
    }

    task.deps.push(dep_id.to_string());
    task.deps.sort();
    task.updated_at = task::now_iso();
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
fn dep_remove(store: &Store, task_id: &str, dep_id: &str, json: bool) -> Result<u8, String> {
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
    task.updated_at = task::now_iso();
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
fn task_update(
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
            if task::graph::would_dep_cycle(&sim, id, dep) {
                return Err(format!(
                    "adding dependency `{id}` -> `{dep}` would create a cycle"
                ));
            }
        }
    }

    task.updated_at = task::now_iso();
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
fn task_delete(store: &Store, id: &str, json: bool) -> Result<u8, String> {
    let all = store.load_all().map_err(|e| format!("store error: {e}"))?;
    if !all.iter().any(|t| t.id == id) {
        eprintln!("task not found: {id}");
        return Ok(1);
    }
    let children = task::graph::children_of(&all, id);
    let dependents = task::graph::dependents(&all, id);
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
fn task_list(
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
fn task_show(store: &Store, id: &str, json: bool) -> Result<u8, String> {
    let all = store.load_all().map_err(|e| format!("store error: {e}"))?;
    let Some(task) = all.iter().find(|t| t.id == id) else {
        eprintln!("task not found: {id}");
        return Ok(1);
    };
    if json {
        let children = task::graph::children_of(&all, &task.id);
        let dependents = task::graph::dependents(&all, &task.id);
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
fn show_cmd(id: &str, json: bool) -> Result<u8, String> {
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    let store = Store::new(&config.data_dir);
    task_show(&store, id, json)
}

/// Print full task detail with computed relationships.
fn print_task_detail(task: &Task, all: &[Task]) {
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
            task::runner::MAX_ATTEMPTS
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

    let children = task::graph::children_of(all, &task.id);
    let dependents = task::graph::dependents(all, &task.id);

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
fn parse_status(s: &str) -> Result<TaskStatus, String> {
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
fn parse_kind(s: &str) -> Result<TaskKind, String> {
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
fn plan_cmd(dot: bool, json: bool) -> Result<u8, String> {
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
/// (`task::runner::MAX_ATTEMPTS`) is exhausted.
fn run_cmd(id: &str) -> Result<u8, String> {
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
        if task::runner::runner_alive(&task_dir) {
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
            task::runner::MAX_ATTEMPTS
        );
    }

    // Deps gate: refuse blocked task without spawning a worker.
    if !task::graph::is_ready(
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
    running.updated_at = task::now_iso();
    store
        .append(&running)
        .map_err(|e| format!("store error: {e}"))?;

    // Runner takes over from here; its outcome decides the store flip.
    let outcome = match task::runner::run(
        &task::runner::Ctx {
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
                task::runner::RunStatus::Done => "done",
                task::runner::RunStatus::Failed => "failed",
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

    if outcome.status == task::runner::RunStatus::Done {
        println!("done {id}");
        Ok(0)
    } else {
        eprintln!("run failed: {}", outcome.summary);
        if updated.attempts >= task::runner::MAX_ATTEMPTS {
            eprintln!(
                "retry limit reached ({} failed attempts); reset with `cli task update {id} --attempts 0`",
                updated.attempts
            );
        } else {
            eprintln!(
                "failed attempts: {}/{}; retry with `cli run {id}`",
                updated.attempts,
                task::runner::MAX_ATTEMPTS
            );
        }
        Ok(1)
    }
}

/// Persist one finished run attempt (#47): flip status to Done/Failed,
/// bump `attempts` on failure, and append the attempt (number, outcome,
/// reason, timestamp) to `<task_dir>/attempts.jsonl`.
fn persist_run_outcome(
    store: &Store,
    data_dir: &std::path::Path,
    pre: &Task,
    outcome: &task::runner::RunOutcome,
) -> Result<Task, String> {
    let attempt = pre.attempts + 1;
    let mut updated = pre.clone();
    updated.status = match outcome.status {
        task::runner::RunStatus::Done => TaskStatus::Done,
        task::runner::RunStatus::Failed => {
            updated.attempts = attempt;
            TaskStatus::Failed
        }
    };
    updated.updated_at = task::now_iso();
    record_attempt(data_dir, &pre.id, attempt, outcome);
    store
        .append(&updated)
        .map_err(|e| format!("store error: {e}"))?;
    Ok(updated)
}

/// Append one attempt record to `attempts.jsonl` (#47 evidence).
/// Best-effort like the event log: a failing evidence write warns on stderr
/// and never breaks the run or the store flip.
fn record_attempt(
    data_dir: &std::path::Path,
    task_id: &str,
    attempt: u32,
    outcome: &task::runner::RunOutcome,
) {
    let task_dir = data_dir.join("tasks").join(task_id);
    if let Err(e) = std::fs::create_dir_all(&task_dir) {
        eprintln!("warning: could not create task context dir: {e}");
        return;
    }
    let rec = serde_json::json!({
        "ts": task::now_iso(),
        "attempt": attempt,
        "outcome": match outcome.status {
            task::runner::RunStatus::Done => "done",
            task::runner::RunStatus::Failed => "failed",
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
fn steer_cmd(id: &str, message: &[String], json: bool) -> Result<u8, String> {
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
fn abort_cmd(id: &str, json: bool) -> Result<u8, String> {
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
        task.updated_at = task::now_iso();
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
fn compact_cmd(json: bool) -> Result<u8, String> {
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
fn init_cmd(json: bool) -> Result<u8, String> {
    let dir = std::env::current_dir().map_err(|e| format!("cwd error: {e}"))?;
    init_cmd_at(&dir, json)
}

/// init internals, testable with an explicit directory.
fn init_cmd_at(dir: &std::path::Path, json: bool) -> Result<u8, String> {
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
    task::template::write_default_templates(&oi_dir).map_err(|e| format!("task templates: {e}"))?;
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

/// `ready` -- list open tasks whose deps are all done, sorted by priority then id.
fn ready_cmd(json: bool) -> Result<u8, String> {
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    let store = Store::new(&config.data_dir);
    let all = store.load_all().map_err(|e| format!("store error: {e}"))?;
    let map: std::collections::HashMap<String, Task> =
        all.iter().map(|t| (t.id.clone(), t.clone())).collect();

    let mut ready: Vec<&Task> = all
        .iter()
        .filter(|t| t.status == TaskStatus::Open && task::graph::is_ready(&map, &t.id))
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
fn blocked_cmd(json: bool) -> Result<u8, String> {
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
fn status_glyph(t: &Task, map: &std::collections::HashMap<String, Task>) -> &'static str {
    match t.status {
        TaskStatus::Done => "✓",
        TaskStatus::InProgress => "◐",
        TaskStatus::Failed => "✗",
        TaskStatus::Open => {
            if task::graph::is_ready(map, &t.id) {
                "○"
            } else {
                "●"
            }
        }
    }
}

/// cx task_line format: `<icon> <id> ● P<priority> <title>`.
fn task_line(t: &Task, map: &std::collections::HashMap<String, Task>) -> String {
    format!(
        "{} {} ● P{} {}",
        status_glyph(t, map),
        t.id,
        t.priority,
        t.title
    )
}

/// Status legend footer, matching bd/cx output conventions.
fn status_legend() -> &'static str {
    "Status: ○ open  ◐ in_progress  ✗ failed  ● blocked  ✓ done\n"
}

/// Render the task tree as an indented plan view (roots first, children
/// nested under their parent with box-drawing prefixes), cx/bd style.
///
/// Tasks whose `parent` is missing (dangling) or `None` are treated as roots.
/// A visited set guards against parent cycles in malformed stores.
fn render_plan(tasks: &[Task]) -> String {
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
                    if *e == 0 {
                        if let Some(t) = kids.iter().find(|k| k.id == m) {
                            queue.push(t);
                        }
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
fn suggest_next(tasks: &[Task], done_id: &str) -> Vec<String> {
    use std::collections::HashMap;
    let map: HashMap<String, Task> = tasks.iter().map(|t| (t.id.clone(), t.clone())).collect();
    tasks
        .iter()
        .filter(|t| {
            t.status == TaskStatus::Open
                && t.deps.iter().any(|d| d == done_id)
                && task::graph::is_ready(&map, &t.id)
        })
        .map(|t| t.id.clone())
        .collect()
}

/// Render the task graph as Graphviz DOT.
///
/// Dependency edges are solid (`dep -> task`); parent→child edges are dotted
/// with `arrowhead=none`. Nodes are colored by status.
fn render_dot(tasks: &[Task]) -> String {
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
fn board_cmd(json: bool) -> Result<u8, String> {
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
                if task::graph::is_ready(&map, &t.id) {
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
fn template_list_cmd(json: bool) -> Result<u8, String> {
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    let templates = task::template::load_all_templates(&config.data_dir)
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
                        task::template::TemplateKind::Phase => "phase",
                        task::template::TemplateKind::Step => "step",
                    },
                    "tasks": t.tasks.iter().map(|x| x.key.clone()).collect::<Vec<_>>(),
                })
            })
            .collect();
        print_json(&list);
    } else {
        for t in &templates {
            let kind = match t.kind {
                task::template::TemplateKind::Phase => "phase",
                task::template::TemplateKind::Step => "step",
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
fn template_apply_cmd(
    store: &Store,
    name: &str,
    topic: &str,
    parent: Option<String>,
    json: bool,
) -> Result<u8, String> {
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    let ids = task::template::apply(store, &config.data_dir, name, topic, parent)
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
fn spec_list_cmd(json: bool) -> Result<u8, String> {
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
fn spec_new_cmd(
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
fn spec_check_cmd(kind: &str, file: &str, json: bool) -> Result<u8, String> {
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
fn spec_view_cmd(file: &str) -> Result<u8, String> {
    let doc = std::fs::read_to_string(file).map_err(|e| format!("read {}: {e}", file))?;
    use std::io::Write;
    print!("{doc}");
    std::io::stdout().flush().ok();
    Ok(0)
}

/// `pr render <task-id>` subcommand: render a task subtree (topic → phase →
/// steps, sibling order from deps) as a PR Construction plan checkbox list.
fn pr_render_cmd(id: &str, json: bool) -> Result<u8, String> {
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
        let mut kids: Vec<&Task> = all
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
                    if *e == 0 {
                        if let Some(t) = kids.iter().find(|k| k.id == m) {
                            queue.push(t);
                        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialize tests: store writes go to distinct temp dirs, but stdout is
    // process-global and some tests capture it.
    static LOCK: Mutex<()> = Mutex::new(());

    fn tmp_store(tag: &str) -> Store {
        let dir =
            std::env::temp_dir().join(format!("omenic-cli-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Store::new(&dir)
    }

    #[test]
    fn add_creates_task() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("add1");
        task_add(
            &store,
            &["write design doc".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let t = store.load_task("write design doc").unwrap().unwrap();
        assert_eq!(t.status, TaskStatus::Open);
        assert_eq!(t.kind, TaskKind::Task);
        assert_eq!(t.parent, None);
        assert_eq!(t.deps, Vec::<String>::new());
    }

    #[test]
    fn add_multiple_titles() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("addmulti");
        let args = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        task_add(&store, &args, None, None, None, None, None, false).unwrap();
        assert_eq!(store.load_all().unwrap().len(), 3);
    }

    #[test]
    fn add_with_parent() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("addparent");
        task_add(
            &store,
            &["child".to_string()],
            Some("schemav1".to_string()),
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let t = store.load_task("child").unwrap().unwrap();
        assert_eq!(t.parent.as_deref(), Some("schemav1"));
    }

    #[test]
    fn done_updates_status_and_timestamp() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("done");
        task_add(
            &store,
            &["t1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let before = store.load_task("t1").unwrap().unwrap().updated_at;
        task_done(&store, "t1", false).unwrap();
        let after = store.load_task("t1").unwrap().unwrap();
        assert_eq!(after.status, TaskStatus::Done);
        // updated_at must not regress; same-second writes may be equal.
        assert!(after.updated_at >= before);
    }

    #[test]
    fn done_missing_task_exits_1() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("donemissing");
        let code = task_done(&store, "nope", false).unwrap();
        assert_eq!(code, 1);
    }

    #[test]
    fn status_missing_task_exits_1() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("statusmissing");
        let code = task_status(&store, "nope", false).unwrap();
        assert_eq!(code, 1);
    }

    #[test]
    fn add_without_title_errors() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("addnoargs");
        let r = task_add(&store, &[], None, None, None, None, None, false);
        assert!(r.is_err());
    }

    #[test]
    fn unknown_subcommand_errors() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let r = Cli::try_parse_from(["oi", "task", "bogus"]);
        assert!(r.is_err());
    }

    #[test]
    fn now_iso_format() {
        let s = task::now_iso();
        assert_eq!(s.len(), 20); // 2026-08-10T12:34:56Z
        assert!(s.ends_with('Z'));
        assert!(s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-' && s.as_bytes()[10] == b'T');
    }
    fn mk_task(id: &str, parent: Option<&str>, status: TaskStatus) -> Task {
        let now = "2026-08-10T00:00:00Z".to_string();
        Task {
            id: id.to_string(),
            title: id.to_string(),
            kind: TaskKind::Task,
            status,
            attempts: 0,
            priority: 2,
            parent: parent.map(|p| p.to_string()),
            deps: vec![],
            description: String::new(),
            acceptance: String::new(),
            created_at: now.clone(),
            updated_at: now,
        }
    }

    #[test]
    fn plan_nested_tree_indentation() {
        let tasks = vec![
            mk_task("dev-shell", None, TaskStatus::Open),
            mk_task("scheme-workflow-01", Some("dev-shell"), TaskStatus::Open),
            mk_task("imp-cli", Some("scheme-workflow-01"), TaskStatus::Open),
            mk_task("imp-rpc", Some("scheme-workflow-01"), TaskStatus::Open),
            mk_task("scheme-workflow-02", Some("dev-shell"), TaskStatus::Done),
        ];
        let expected = "\
○ dev-shell ● P2 dev-shell
├─ ○ scheme-workflow-01 ● P2 scheme-workflow-01
│  ├─ ○ imp-cli ● P2 imp-cli
│  └─ ○ imp-rpc ● P2 imp-rpc
└─ ✓ scheme-workflow-02 ● P2 scheme-workflow-02
Status: ○ open  ◐ in_progress  ✗ failed  ● blocked  ✓ done
";
        assert_eq!(render_plan(&tasks), expected);
    }

    #[test]
    fn plan_empty_store() {
        assert_eq!(render_plan(&[]), "(no tasks)\n");
    }

    #[test]
    fn plan_dangling_parent_is_root() {
        // parent id that doesn't exist in the store → treated as a root
        let tasks = vec![
            mk_task("a", Some("ghost"), TaskStatus::Open),
            mk_task("b", None, TaskStatus::Open),
        ];
        let out = render_plan(&tasks);
        assert!(out.starts_with("○ a ● P2 a\n"));
        assert!(out.contains("○ b ● P2 b"));
    }

    #[test]
    fn plan_cycle_does_not_hang() {
        let tasks = vec![
            mk_task("a", Some("b"), TaskStatus::Open),
            mk_task("b", Some("a"), TaskStatus::Open),
        ];
        let out = render_plan(&tasks);
        // Both appear exactly once; renderer terminates.
        assert_eq!(out.matches("○ a ● P2 a").count(), 1);
        assert_eq!(out.matches("○ b ● P2 b").count(), 1);
    }

    #[test]
    fn plan_status_rendering() {
        let tasks = vec![
            mk_task("t-open", None, TaskStatus::Open),
            mk_task("t-ip", None, TaskStatus::InProgress),
            mk_task("t-done", None, TaskStatus::Done),
        ];
        let out = render_plan(&tasks);
        assert!(out.contains("○ t-open ● P2 t-open"));
        assert!(out.contains("◐ t-ip ● P2 t-ip"));
        assert!(out.contains("✓ t-done ● P2 t-done"));
    }

    #[test]
    fn run_command_parse_errors_on_missing_id() {
        let r = Cli::try_parse_from(["oi", "run"]).is_err();
        assert!(r); // needs <task-id>
    }

    #[test]
    fn steer_command_parse_and_note() {
        // non-empty message required after id; bare steer errors
        // steer with no message: clap allows it (empty vec), but steer_cmd should error
        let r = Cli::try_parse_from(["oi", "steer", "t-1"]);
        assert!(r.is_ok()); // parsing succeeds; steer_cmd handles empty message
        // with msg should hit the handle (but not fail dispatch parse)
        let r = Cli::try_parse_from(["oi", "steer", "t-1", "keep chipping"]);
        assert!(r.is_ok());
    }

    #[test]
    fn abort_command_parse_needs_id() {
        let r = Cli::try_parse_from(["oi", "abort"]).is_err();
        assert!(r);
    }

    // --- #50: unknown flags rejected, not swallowed as title ---
    #[test]
    fn add_unknown_flag_rejected() {
        let r = Cli::try_parse_from(["oi", "task", "add", "-t", "foo"]);
        assert!(r.is_err(), "unknown flag -t must be rejected");
    }

    #[test]
    fn add_unknown_long_flag_rejected() {
        let r = Cli::try_parse_from(["oi", "task", "add", "--bogus", "x"]);
        assert!(r.is_err());
    }

    // --- #42: --deps and --acceptance ---

    #[test]
    fn add_with_deps() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("deps42");
        // Create prerequisite tasks first
        task_add(
            &store,
            &["prereq-a".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["prereq-b".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        // Create dependent task
        task_add(
            &store,
            &["dependent".to_string()],
            None,
            Some("prereq-a,prereq-b".to_string()),
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let t = store.load_task("dependent").unwrap().unwrap();
        assert_eq!(t.deps, vec!["prereq-a", "prereq-b"]);
    }

    #[test]
    fn add_with_acceptance() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("acc42");
        task_add(
            &store,
            &["task-x".to_string()],
            None,
            None,
            Some("all tests pass".to_string()),
            None,
            None,
            false,
        )
        .unwrap();
        let t = store.load_task("task-x").unwrap().unwrap();
        assert_eq!(t.acceptance, "all tests pass");
    }

    #[test]
    fn add_deps_nonexistent_rejected() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("deps404");
        let r = task_add(
            &store,
            &["t1".to_string()],
            None,
            Some("ghost".to_string()),
            None,
            None,
            None,
            false,
        );
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("does not exist"));
    }

    #[test]
    fn add_self_dep_rejected() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("selfdep");
        let r = task_add(
            &store,
            &["s1".to_string()],
            None,
            Some("s1".to_string()),
            None,
            None,
            None,
            false,
        );
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("cannot depend on itself"));
    }

    #[test]
    fn add_deps_cycle_rejected() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("cycle42");
        // a exists, b depends on a
        task_add(
            &store,
            &["a".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["b".to_string()],
            None,
            Some("a".to_string()),
            None,
            None,
            None,
            false,
        )
        .unwrap();
        // re-create a with deps=[b] → a→b→a cycle
        let r = task_add(
            &store,
            &["a".to_string()],
            None,
            Some("b".to_string()),
            None,
            None,
            None,
            false,
        );
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("cycle"));
    }

    #[test]
    fn add_deps_and_acceptance_combined() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("combo42");
        task_add(
            &store,
            &["p1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["child".to_string()],
            Some("parent-epic".to_string()),
            Some("p1".to_string()),
            Some("done when p1 is verified".to_string()),
            None,
            None,
            false,
        )
        .unwrap();
        let t = store.load_task("child").unwrap().unwrap();
        assert_eq!(t.deps, vec!["p1"]);
        assert_eq!(t.acceptance, "done when p1 is verified");
        assert_eq!(t.parent.as_deref(), Some("parent-epic"));
    }

    // --- #179: dep add / dep remove ---

    fn dep_store(tag: &str) -> (Store, [String; 3]) {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store(tag);
        task_add(
            &store,
            &["t1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["t2".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["t3".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let ids = ["t1".to_string(), "t2".to_string(), "t3".to_string()];
        (store, ids)
    }

    #[test]
    fn dep_add_creates_edge() {
        let (store, ids) = dep_store("depadd1");
        let r = dep_add(&store, "t1", "t2", false);
        assert!(r.is_ok());
        let t = store.load_task(&ids[0]).unwrap().unwrap();
        assert_eq!(t.deps, vec!["t2"]);
    }

    #[test]
    fn dep_add_includes_dep_in_status() {
        let (store, _ids) = dep_store("depadd_status");
        dep_add(&store, "t1", "t2", false).unwrap();
        let t = store.load_task("t1").unwrap().unwrap();
        assert!(t.deps.contains(&"t2".to_string()));
    }

    #[test]
    fn dep_add_self_dep_rejected() {
        let (store, _ids) = dep_store("depself");
        let r = dep_add(&store, "t1", "t1", false);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("cannot depend on itself"));
    }

    #[test]
    fn dep_add_duplicate_rejected() {
        let (store, _ids) = dep_store("depdup");
        dep_add(&store, "t1", "t2", false).unwrap();
        let r = dep_add(&store, "t1", "t2", false);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("already exists"));
    }

    #[test]
    fn dep_add_nonexistent_dep_rejected() {
        let (store, _ids) = dep_store("dep404");
        let r = dep_add(&store, "t1", "ghost", false);
        // dep_id doesn't exist → eprintln + Ok(1)
        assert!(r.is_ok());
        assert_eq!(r.unwrap(), 1);
    }

    #[test]
    fn dep_add_nonexistent_task_rejected() {
        let (store, _ids) = dep_store("dep404task");
        let r = dep_add(&store, "ghost", "t2", false);
        assert!(r.is_ok());
        assert_eq!(r.unwrap(), 1);
    }

    #[test]
    fn dep_add_cycle_rejected() {
        let (store, _ids) = dep_store("depcycle");
        // t1 → t2
        dep_add(&store, "t1", "t2", false).unwrap();
        // t2 → t3
        dep_add(&store, "t2", "t3", false).unwrap();
        // t3 → t1 would create cycle: t1→t2→t3→t1
        let r = dep_add(&store, "t3", "t1", false);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("cycle"));
    }

    #[test]
    fn dep_add_keeps_deps_sorted() {
        let (store, _ids) = dep_store("depsort");
        dep_add(&store, "t1", "t3", false).unwrap();
        dep_add(&store, "t1", "t2", false).unwrap();
        let t = store.load_task("t1").unwrap().unwrap();
        assert_eq!(t.deps, vec!["t2", "t3"]);
    }

    #[test]
    fn dep_add_updates_timestamp() {
        let (store, _ids) = dep_store("dep_ts");
        let before = store.load_task("t1").unwrap().unwrap().updated_at;
        dep_add(&store, "t1", "t2", false).unwrap();
        let after = store.load_task("t1").unwrap().unwrap();
        assert!(after.updated_at >= before);
    }

    #[test]
    fn dep_remove_deletes_edge() {
        let (store, _ids) = dep_store("depremove1");
        dep_add(&store, "t1", "t2", false).unwrap();
        let r = dep_remove(&store, "t1", "t2", false);
        assert!(r.is_ok());
        let t = store.load_task("t1").unwrap().unwrap();
        assert!(!t.deps.contains(&"t2".to_string()));
        assert!(t.deps.is_empty());
    }

    #[test]
    fn dep_remove_not_a_dep_rejected() {
        let (store, _ids) = dep_store("depremove_missing");
        dep_add(&store, "t1", "t2", false).unwrap();
        // t3 was never a dep of t1
        let r = dep_remove(&store, "t1", "t3", false);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("not found"));
    }

    #[test]
    fn dep_remove_missing_task_rejected() {
        let (store, _ids) = dep_store("depremove_404");
        let r = dep_remove(&store, "ghost", "t2", false);
        assert!(r.is_ok());
        assert_eq!(r.unwrap(), 1);
    }

    #[test]
    fn dep_remove_preserves_other_deps() {
        let (store, _ids) = dep_store("depremove_keep");
        dep_add(&store, "t1", "t2", false).unwrap();
        dep_add(&store, "t1", "t3", false).unwrap();
        dep_remove(&store, "t1", "t2", false).unwrap();
        let t = store.load_task("t1").unwrap().unwrap();
        assert_eq!(t.deps, vec!["t3"]);
    }

    #[test]
    fn dep_cmd_unknown_sub_rejected() {
        let r = Cli::try_parse_from(["oi", "dep", "bogus"]);
        assert!(r.is_err());
    }

    #[test]
    fn dep_cmd_no_sub_rejected() {
        let r = Cli::try_parse_from(["oi", "dep"]);
        assert!(r.is_err());
    }

    #[test]
    fn dep_add_wrong_arg_count_rejected() {
        let r = Cli::try_parse_from(["oi", "dep", "add", "t1"]);
        assert!(r.is_err());
    }

    #[test]
    fn dep_remove_wrong_arg_count_rejected() {
        let r = Cli::try_parse_from(["oi", "dep", "remove", "t1"]);
        assert!(r.is_err());
    }

    // --- #178: task update / delete / list / show ---

    #[test]
    fn update_status() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("upd_status");
        task_add(
            &store,
            &["t1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_update(
            &store,
            "t1",
            None,
            None,
            Some("in_progress".to_string()),
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let t = store.load_task("t1").unwrap().unwrap();
        assert_eq!(t.status, TaskStatus::InProgress);
    }

    #[test]
    fn update_title() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("upd_title");
        task_add(
            &store,
            &["t1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_update(
            &store,
            "t1",
            Some("new title".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let t = store.load_task("t1").unwrap().unwrap();
        assert_eq!(t.title, "new title");
    }

    #[test]
    fn update_acceptance() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("upd_acc");
        task_add(
            &store,
            &["t1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_update(
            &store,
            "t1",
            None,
            None,
            None,
            None,
            Some("all pass".to_string()),
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let t = store.load_task("t1").unwrap().unwrap();
        assert_eq!(t.acceptance, "all pass");
    }

    #[test]
    fn update_deps() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("upd_deps");
        task_add(
            &store,
            &["a".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["b".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_update(
            &store,
            "b",
            None,
            None,
            None,
            Some("a".to_string()),
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let t = store.load_task("b").unwrap().unwrap();
        assert_eq!(t.deps, vec!["a"]);
    }

    #[test]
    fn update_missing_task() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("upd_404");
        let code = task_update(
            &store, "nope", None, None, None, None, None, None, None, None, false,
        )
        .unwrap();
        assert_eq!(code, 1);
    }

    #[test]
    fn update_invalid_status() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("upd_badstatus");
        task_add(
            &store,
            &["t1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let r = task_update(
            &store,
            "t1",
            None,
            None,
            Some("bogus".to_string()),
            None,
            None,
            None,
            None,
            None,
            false,
        );
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("invalid status"));
    }

    #[test]
    fn update_missing_task_returns_1() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("upd_noargs");
        let r = task_update(
            &store,
            "nonexistent",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
        );
        assert_eq!(r.unwrap(), 1);
    }

    #[test]
    fn update_self_dep_rejected() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("upd_selfdep");
        task_add(
            &store,
            &["t1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let r = task_update(
            &store,
            "t1",
            None,
            None,
            None,
            Some("t1".to_string()),
            None,
            None,
            None,
            None,
            false,
        );
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("cannot depend on itself"));
    }

    #[test]
    fn update_deps_nonexistent_rejected() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("upd_dep404");
        task_add(
            &store,
            &["t1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let r = task_update(
            &store,
            "t1",
            None,
            None,
            None,
            Some("ghost".to_string()),
            None,
            None,
            None,
            None,
            false,
        );
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("does not exist"));
    }

    #[test]
    fn delete_isolated_task() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("del_isolated");
        task_add(
            &store,
            &["orphan".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_delete(&store, "orphan", false).unwrap();
        assert!(store.load_task("orphan").unwrap().is_none());
    }

    #[test]
    fn delete_with_child_rejected() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("del_child");
        task_add(
            &store,
            &["parent".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["child".to_string()],
            Some("parent".to_string()),
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let r = task_delete(&store, "parent", false);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("cannot delete"));
    }

    #[test]
    fn delete_with_dependent_rejected() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("del_dep");
        task_add(
            &store,
            &["a".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["b".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        dep_add(&store, "b", "a", false).unwrap();
        let r = task_delete(&store, "a", false);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("cannot delete"));
    }

    #[test]
    fn delete_missing_task() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("del_404");
        let code = task_delete(&store, "nope", false).unwrap();
        assert_eq!(code, 1);
    }

    #[test]
    fn list_no_filter() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("list_all");
        task_add(
            &store,
            &["a".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["b".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let r = task_list(&store, None, None, None, false);
        assert!(r.is_ok());
    }

    #[test]
    fn list_by_status() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("list_status");
        task_add(
            &store,
            &["open1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["open2".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["done1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_done(&store, "done1", false).unwrap();
        let r = task_list(&store, Some("done".to_string()), None, None, false);
        assert!(r.is_ok());
        // Verify only done tasks in store match
        let done: Vec<Task> = store
            .load_all()
            .unwrap()
            .into_iter()
            .filter(|t| t.status == TaskStatus::Done)
            .collect();
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].id, "done1");
    }

    #[test]
    fn list_by_parent() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("list_parent");
        task_add(
            &store,
            &["root".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["child".to_string()],
            Some("root".to_string()),
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let r = task_list(&store, None, None, Some("none".to_string()), false);
        assert!(r.is_ok());
        let roots: Vec<Task> = store
            .load_all()
            .unwrap()
            .into_iter()
            .filter(|t| t.parent.is_none())
            .collect();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].id, "root");
    }

    #[test]
    fn list_empty_store() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("list_empty");
        let r = task_list(&store, None, None, None, false);
        assert!(r.is_ok());
    }

    #[test]
    fn show_task_detail() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("show_detail");
        task_add(
            &store,
            &["t1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let r = task_show(&store, "t1", false);
        assert!(r.is_ok());
    }

    #[test]
    fn show_task_with_children_and_deps() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("show_rels");
        task_add(
            &store,
            &["a".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["b".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        dep_add(&store, "b", "a", false).unwrap();
        task_add(
            &store,
            &["child".to_string()],
            Some("a".to_string()),
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let r = task_show(&store, "a", false);
        assert!(r.is_ok());
        let all = store.load_all().unwrap();
        let children = task::graph::children_of(&all, "a");
        let dependents = task::graph::dependents(&all, "a");
        assert_eq!(children, vec!["child"]);
        assert_eq!(dependents, vec!["b"]);
    }

    #[test]
    fn show_missing_task() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("show_404");
        let code = task_show(&store, "nope", false).unwrap();
        assert_eq!(code, 1);
    }
    #[test]
    fn show_cmd_top_level_no_arg_errors() {
        // No arg → usage error before Config::load is reached.
        let r = Cli::try_parse_from(["oi", "show"]);
        assert!(r.is_err());
    }

    #[test]
    fn show_cmd_top_level_alias() {
        // show_cmd calls Config::load which needs env; test core logic via task_show instead.
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("show_alias");
        task_add(
            &store,
            &["t1".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        let r = task_show(&store, "t1", false);
        assert!(r.is_ok());
    }
    fn store_file(tag: &str) -> String {
        let path = std::env::temp_dir()
            .join(format!("omenic-cli-test-{tag}-{}", std::process::id()))
            .join("tasks.jsonl");
        std::fs::read_to_string(&path).unwrap_or_default()
    }

    fn mk_task_titled(id: &str, title: &str) -> Task {
        let now = task::now_iso();
        Task {
            id: id.to_string(),
            title: title.to_string(),
            kind: TaskKind::Task,
            status: TaskStatus::Open,
            attempts: 0,
            priority: 2,
            parent: None,
            deps: Vec::new(),
            description: String::new(),
            acceptance: String::new(),
            created_at: now.clone(),
            updated_at: now,
        }
    }

    #[test]
    fn compact_reduces_duplicate_ids() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("compact_dup");
        assert!(store.append(&mk_task_titled("t1", "v1")).is_ok());
        assert!(store.append(&mk_task_titled("t1", "v2")).is_ok());
        assert!(store.append(&mk_task_titled("t1", "v3")).is_ok());
        assert!(store.append(&mk_task_titled("t1", "v4")).is_ok());
        // load_all already dedupes (latest-wins).
        let before = store.load_all().unwrap();
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].title, "v4");
        // File still holds all 4 lines physically.
        assert_eq!(store_file("compact_dup").lines().count(), 4);
        assert!(store.compact().is_ok());
        // After compact: 1 logical task, latest title, 1 physical line.
        let after = store.load_all().unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].title, "v4");
        assert_eq!(store_file("compact_dup").lines().count(), 1);
    }

    #[test]
    fn compact_drops_tombstones() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("compact_tomb");
        task_add(
            &store,
            &["alpha".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["beta".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_delete(&store, "beta", false).unwrap();
        // File: 2 task lines + 1 tombstone line.
        assert_eq!(store_file("compact_tomb").lines().count(), 3);
        assert!(store_file("compact_tomb").contains("tombstone"));
        assert_eq!(store.load_all().unwrap().len(), 1);
        assert!(store.compact().is_ok());
        // After compact: only alpha remains; no tombstone line in file.
        let after = store.load_all().unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].id, "alpha");
        let file = store_file("compact_tomb");
        assert!(!file.contains("tombstone"));
        assert_eq!(file.lines().count(), 1);
    }

    #[test]
    fn compact_empty_store() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("compact_empty");
        assert!(store.load_all().unwrap().is_empty());
        assert!(store.compact().is_ok());
        assert!(store.load_all().unwrap().is_empty());
    }

    #[test]
    fn compact_preserves_data() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("compact_data");
        task_add(
            &store,
            &["base".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        // second task depends on base, with acceptance text.
        task_add(
            &store,
            &["child".to_string()],
            None,
            Some("base".to_string()),
            Some("all tests green".to_string()),
            None,
            None,
            false,
        )
        .unwrap();
        let before = store.load_all().unwrap();
        assert_eq!(before.len(), 2);
        assert!(store.compact().is_ok());
        let after = store.load_all().unwrap();
        assert_eq!(after.len(), 2);
        let child = after.iter().find(|t| t.id == "child").unwrap();
        assert_eq!(child.deps, vec!["base".to_string()]);
        assert_eq!(child.acceptance, "all tests green");
    }

    // --- #183: cli init ---

    #[test]
    fn init_creates_data_dir_and_config() {
        let dir = std::env::temp_dir().join(format!(
            "omenic-cli-test-init-create-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        init_cmd_at(&dir, false).unwrap();
        assert!(dir.join(".oi").is_dir());
        assert!(dir.join(".oi/config.toml").is_file());
        assert!(dir.join(".oi/specs").is_dir());
        let content = std::fs::read_to_string(dir.join(".oi/config.toml")).unwrap();
        assert!(content.contains("omp_path = \"omp\""));
        assert!(content.contains("data_dir = \"./.oi\""));
        assert!(content.contains("model = \"default\""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn init_idempotent() {
        let dir =
            std::env::temp_dir().join(format!("omenic-cli-test-init-idem-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        init_cmd_at(&dir, false).unwrap();
        let toml = std::fs::read_to_string(dir.join(".oi/config.toml")).unwrap();
        init_cmd_at(&dir, false).unwrap();
        // Not overwritten.
        assert_eq!(
            toml,
            std::fs::read_to_string(dir.join(".oi/config.toml")).unwrap()
        );
        assert!(dir.join(".oi").is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn init_partial_existing() {
        let dir = std::env::temp_dir().join(format!(
            "omenic-cli-test-init-partial-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".oi")).unwrap();
        init_cmd_at(&dir, false).unwrap();
        // config.toml + specs created, .oi already there.
        assert!(dir.join(".oi/config.toml").is_file());
        assert!(dir.join(".oi/specs").is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- #181: cli ready / cli blocked ---

    /// Helper: build a store with tasks from a spec of (id, priority, deps).
    fn make_ready_store(tag: &str, specs: &[(&str, u8, &[&str])]) -> Store {
        let store = tmp_store(tag);
        for &(id, prio, deps) in specs {
            let deps_opt = if deps.is_empty() {
                None
            } else {
                Some(deps.join(","))
            };
            task_add(
                &store,
                &[id.to_string()],
                None,
                deps_opt,
                None,
                Some(prio),
                None,
                false,
            )
            .unwrap();
        }
        store
    }

    #[test]
    fn ready_shows_only_unblocked_open() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = make_ready_store(
            "ready1",
            &[("A", 2, &[]), ("B", 2, &["A"]), ("C", 2, &["A"])],
        );
        let all = store.load_all().unwrap();
        let map: std::collections::HashMap<String, Task> =
            all.iter().map(|t| (t.id.clone(), t.clone())).collect();
        let ready: Vec<&str> = all
            .iter()
            .filter(|t| t.status == TaskStatus::Open && task::graph::is_ready(&map, &t.id))
            .map(|t| t.id.as_str())
            .collect();
        assert_eq!(ready, vec!["A"]);
    }

    #[test]
    fn ready_after_done_shows_unblocked() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = make_ready_store(
            "ready2",
            &[("A", 2, &[]), ("B", 2, &["A"]), ("C", 2, &["A"])],
        );
        task_done(&store, "A", false).unwrap();
        let all = store.load_all().unwrap();
        let map: std::collections::HashMap<String, Task> =
            all.iter().map(|t| (t.id.clone(), t.clone())).collect();
        let mut ready: Vec<&str> = all
            .iter()
            .filter(|t| t.status == TaskStatus::Open && task::graph::is_ready(&map, &t.id))
            .map(|t| t.id.as_str())
            .collect();
        ready.sort();
        assert_eq!(ready, vec!["B", "C"]);
    }

    #[test]
    fn ready_empty() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("ready_empty");
        let all = store.load_all().unwrap();
        let map: std::collections::HashMap<String, Task> =
            all.iter().map(|t| (t.id.clone(), t.clone())).collect();
        let ready: Vec<&Task> = all
            .iter()
            .filter(|t| t.status == TaskStatus::Open && task::graph::is_ready(&map, &t.id))
            .collect();
        assert!(ready.is_empty());
        // The command prints "(no ready tasks)" when empty.
    }

    #[test]
    fn blocked_shows_blocked_with_reasons() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = make_ready_store("blocked1", &[("A", 2, &[]), ("B", 2, &["A"])]);
        let all = store.load_all().unwrap();
        let map: std::collections::HashMap<String, Task> =
            all.iter().map(|t| (t.id.clone(), t.clone())).collect();
        // B is blocked by A (A is open, not done).
        let blocked: Vec<(String, Vec<String>)> = all
            .iter()
            .filter_map(|t| {
                if t.status == TaskStatus::Done {
                    return None;
                }
                let unmet: Vec<String> = t
                    .deps
                    .iter()
                    .filter(|dep| map.get(*dep).map_or(true, |d| d.status != TaskStatus::Done))
                    .cloned()
                    .collect();
                if unmet.is_empty() {
                    None
                } else {
                    Some((t.id.clone(), unmet))
                }
            })
            .collect();
        // B should be blocked by A. A has no deps → not blocked.
        let b = blocked.iter().find(|(id, _)| id == "B").unwrap();
        assert_eq!(b.1, vec!["A".to_string()]);
        assert!(!blocked.iter().any(|(id, _)| id == "A"));
    }

    #[test]
    fn blocked_empty() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // A has no deps → not blocked. All ready → "(no blocked tasks)".
        let store = make_ready_store("blocked_empty", &[("A", 2, &[])]);
        let all = store.load_all().unwrap();
        let map: std::collections::HashMap<String, Task> =
            all.iter().map(|t| (t.id.clone(), t.clone())).collect();
        let blocked: Vec<&Task> = all
            .iter()
            .filter_map(|t| {
                if t.status == TaskStatus::Done {
                    return None;
                }
                let unmet = t
                    .deps
                    .iter()
                    .filter(|dep| map.get(*dep).map_or(true, |d| d.status != TaskStatus::Done));
                if unmet.count() == 0 { None } else { Some(t) }
            })
            .collect();
        assert!(blocked.is_empty());
    }

    #[test]
    fn ready_sorted_by_priority() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Three root tasks with different priorities.
        let store = make_ready_store(
            "ready_sort",
            &[("low", 2, &[]), ("high", 0, &[]), ("mid", 1, &[])],
        );
        let all = store.load_all().unwrap();
        let map: std::collections::HashMap<String, Task> =
            all.iter().map(|t| (t.id.clone(), t.clone())).collect();
        let mut ready: Vec<&Task> = all
            .iter()
            .filter(|t| t.status == TaskStatus::Open && task::graph::is_ready(&map, &t.id))
            .collect();
        ready.sort_by(|a, b| a.priority.cmp(&b.priority).then(a.id.cmp(&b.id)));
        let ids: Vec<&str> = ready.iter().map(|t| t.id.as_str()).collect();
        // P0 (high) before P1 (mid) before P2 (low).
        assert_eq!(ids, vec!["high", "mid", "low"]);
    }

    // --- #184: plan --dot + done suggest-next ---

    #[test]
    fn plan_dot_outputs_valid_dot() {
        let mut a = mk_task("A", None, TaskStatus::Open);
        a.deps = vec![];
        let mut b = mk_task("B", None, TaskStatus::Open);
        b.deps = vec!["A".to_string()];
        let tasks = vec![a, b];
        let dot = render_dot(&tasks);
        assert!(dot.starts_with("digraph omenic {\n"));
        assert!(dot.contains("\"A\""));
        assert!(dot.contains("\"B\""));
        // B depends on A → edge A -> B
        assert!(dot.contains("\"A\" -> \"B\";"));
        assert!(dot.ends_with("}\n"));
    }

    #[test]
    fn plan_dot_empty() {
        assert_eq!(render_dot(&[]), "digraph omenic {\n}\n");
    }

    #[test]
    fn plan_dot_colors_by_status() {
        let tasks = vec![
            mk_task("t-open", None, TaskStatus::Open),
            mk_task("t-ip", None, TaskStatus::InProgress),
            mk_task("t-done", None, TaskStatus::Done),
        ];
        let dot = render_dot(&tasks);
        assert!(dot.contains("fillcolor=\"#e8f4fd\""), "open color");
        assert!(dot.contains("fillcolor=\"#fff3cd\""), "in_progress color");
        assert!(dot.contains("fillcolor=\"#d4edda\""), "done color");
    }

    #[test]
    fn plan_dot_parent_child_dotted_edge() {
        let tasks = vec![
            mk_task("parent", None, TaskStatus::Open),
            mk_task("child", Some("parent"), TaskStatus::Open),
        ];
        let dot = render_dot(&tasks);
        assert!(
            dot.contains("\"parent\" -> \"child\" [style=dotted, arrowhead=none]"),
            "parent edge styled dotted"
        );
    }

    // suggest_next is tested via the pure helper to avoid stdout capture.

    #[test]
    fn done_suggest_next_shows_unblocked() {
        let mut a = mk_task("A", None, TaskStatus::Done);
        a.deps = vec![];
        let mut b = mk_task("B", None, TaskStatus::Open);
        b.deps = vec!["A".to_string()];
        let mut c = mk_task("C", None, TaskStatus::Open);
        c.deps = vec!["A".to_string()];
        let ready = suggest_next(&[a, b, c], "A");
        assert!(ready.contains(&"B".to_string()));
        assert!(ready.contains(&"C".to_string()));
        assert_eq!(ready.len(), 2);
    }

    #[test]
    fn done_suggest_next_none() {
        let a = mk_task("A", None, TaskStatus::Done);
        let b = mk_task("B", None, TaskStatus::Open); // B does NOT depend on A
        let ready = suggest_next(&[a, b], "A");
        assert!(ready.is_empty());
    }

    #[test]
    fn done_suggest_next_partial_deps_not_ready() {
        // B deps=[A, X]; A done but X not → B still blocked.
        let mut a = mk_task("A", None, TaskStatus::Done);
        a.deps = vec![];
        let mut x = mk_task("X", None, TaskStatus::Open);
        x.deps = vec![];
        let mut b = mk_task("B", None, TaskStatus::Open);
        b.deps = vec!["A".to_string(), "X".to_string()];
        let ready = suggest_next(&[a, x, b], "A");
        assert!(ready.is_empty());
    }

    #[test]
    fn done_suggest_next_skips_done_tasks() {
        // B already Done and deps=[A] → not suggested (it's not Open).
        let mut a = mk_task("A", None, TaskStatus::Done);
        a.deps = vec![];
        let mut b = mk_task("B", None, TaskStatus::Done);
        b.deps = vec!["A".to_string()];
        let ready = suggest_next(&[a, b], "A");
        assert!(ready.is_empty());
    }

    #[test]
    fn done_suggest_next_via_store_integration() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("suggest184");
        task_add(
            &store,
            &["A".to_string()],
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
        task_add(
            &store,
            &["B".to_string()],
            None,
            Some("A".to_string()),
            None,
            None,
            None,
            false,
        )
        .unwrap();
        // Mark A done → B should be newly ready.
        task_done(&store, "A", false).unwrap();
        let all = store.load_all().unwrap();
        let ready = suggest_next(&all, "A");
        assert!(ready.contains(&"B".to_string()));
    }

    #[test]
    fn parse_status_accepts_failed() {
        assert_eq!(parse_status("failed"), Ok(TaskStatus::Failed));
        assert!(parse_status("bogus").is_err());
    }

    #[test]
    fn update_resets_attempts() {
        // #47: the documented escape hatch for an exhausted retry budget.
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let store = tmp_store("attempts_reset");
        store.append(&mk_task_titled("t1", "t1")).unwrap();
        task_update(
            &store,
            "t1",
            None,
            None,
            Some("failed".into()),
            None,
            None,
            None,
            None,
            Some(3),
            false,
        )
        .unwrap();
        let t = store.load_task("t1").unwrap().unwrap();
        assert_eq!(t.status, TaskStatus::Failed);
        assert_eq!(t.attempts, 3);
        task_update(
            &store,
            "t1",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(0),
            false,
        )
        .unwrap();
        assert_eq!(store.load_task("t1").unwrap().unwrap().attempts, 0);
    }

    #[test]
    fn persist_failed_attempt_bumps_counter_and_evidence() {
        // #47: a failed run flips to Failed, counts the attempt and leaves
        // the reason + timestamp in attempts.jsonl.
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir =
            std::env::temp_dir().join(format!("omenic-cli-test-persist-f-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(&dir);
        let pre = Task {
            attempts: 2,
            ..mk_task_titled("t1", "t1")
        };
        store.append(&pre).unwrap();

        let outcome = task::runner::RunOutcome {
            status: task::runner::RunStatus::Failed,
            summary: "read_event error: 中文 \"boom\" 💥".into(),
            events_seen: 7,
        };
        let updated = persist_run_outcome(&store, &dir, &pre, &outcome).unwrap();
        assert_eq!(updated.status, TaskStatus::Failed);
        assert_eq!(updated.attempts, 3);

        let stored = store.load_task("t1").unwrap().unwrap();
        assert_eq!(stored.attempts, 3);

        let rec: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("tasks/t1/attempts.jsonl")).unwrap(),
        )
        .unwrap();
        assert_eq!(rec["attempt"], 3);
        assert_eq!(rec["outcome"], "failed");
        assert_eq!(rec["reason"], "read_event error: 中文 \"boom\" 💥");
        assert_eq!(rec["events_seen"], 7);
        assert_eq!(rec["ts"].as_str().unwrap().len(), 20);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_done_attempt_keeps_failure_count() {
        // #47: success does not count as a failure; the record still lands
        // in evidence.
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir =
            std::env::temp_dir().join(format!("omenic-cli-test-persist-d-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(&dir);
        let pre = mk_task_titled("t2", "t2");
        store.append(&pre).unwrap();

        let outcome = task::runner::RunOutcome {
            status: task::runner::RunStatus::Done,
            summary: "all good".into(),
            events_seen: 3,
        };
        let updated = persist_run_outcome(&store, &dir, &pre, &outcome).unwrap();
        assert_eq!(updated.status, TaskStatus::Done);
        assert_eq!(updated.attempts, 0);
        assert_eq!(
            store.load_task("t2").unwrap().unwrap().status,
            TaskStatus::Done
        );

        let rec: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("tasks/t2/attempts.jsonl")).unwrap(),
        )
        .unwrap();
        assert_eq!(rec["attempt"], 1);
        assert_eq!(rec["outcome"], "done");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
