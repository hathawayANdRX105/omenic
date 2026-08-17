//! Runner: orchestration template — the single MVP run flow.
//!
//! Implements §6.1 of the MVP design (`todo/spike/mvp-design.md`): a
//! hardcoded sequential flow from dependency check through worker spawn,
//! brief prompt, event loop, and terminal result collection.
//!
//! Runner does NOT persist task status — the caller is responsible for
//! flipping the store. This keeps the runner testable without touching I/O.

#![allow(dead_code)] // consumed by CLI run/steer/abort in M3.#31
//!
//! Flow:
//! 1. If task missing → NotFound.
//! 2. If any dep open    → Blocked.
//! 3. Worker::new (omp --mode rpc).
//! 4. Assemble brief (description + acceptance + materials path).
//! 5. worker.prompt(brief) — errors map to Failed status.
//! 6. Loop read_event until AgentEnd (or response), printing each event.
//!    Prompt errors or read errors map to Failed.
//! 7. Drop worker; kill via Drop guard.
//! 8. Return RunOutcome { Done|Failed, summary, events_seen }.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

use crate::graph;
use crate::task::{Task, TaskStatus};
use crate::worker::{Worker, WorkerEvent};

/// Terminal outcome of one `run`.
#[derive(Debug, PartialEq)]
pub enum RunStatus {
    /// Agent reached `agent_end` without a fatal prompt/transport error.
    Done,
    /// Prompt or event stream failed (omp exit, idle EOF, JSON error, ...).
    Failed,
}

/// Return value of a finished run. The caller decides what to do with it
/// (e.g. write TaskStatus::done to the store on `Done`).
#[derive(Debug, PartialEq)]
pub struct RunOutcome {
    pub status: RunStatus,
    /// Compact summary (last agent message text or error string).
    pub summary: String,
    /// Distinct WorkerEvents observed during the run.
    pub events_seen: usize,
}

/// Failure surfaced before the run reaches the event loop.
#[derive(Debug, Clone)]
pub enum RunnerError {
    NotFound(String),
    Blocked(String),
    InvalidStatus(String),
}

impl std::fmt::Display for RunnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunnerError::NotFound(id) => write!(f, "task not found: {id}"),
            RunnerError::Blocked(id) => write!(
                f,
                "deps not ready for {id}; blocked until predecessors complete"
            ),
            RunnerError::InvalidStatus(msg) => write!(f, "{msg}"),
        }
    }
}
impl std::error::Error for RunnerError {}

/// Caller-supplied context for the run.
pub struct Ctx {
    /// Path to the omp binary.
    pub omp_path: PathBuf,
    /// All tasks currently in the store (for the deps gate).
    pub tasks: HashMap<String, Task>,
    /// Per-task TaskContext material root — resolved as
    /// `<data_dir>/tasks/<task-id>/`.
    pub data_dir: PathBuf,
}

/// Run one task end-to-end through the hardcoded MVP pipeline.
///
/// Requires `task_id` to exist and its dependencies to be complete. Spawns a
/// fresh `oomp --mode rpc` worker, assembles and prompts the brief, streams
/// agent events to stdout, and returns `RunOutcome`. Never panics; all MPI /
/// transport errors surface as `RunOutcome::Failed`.
pub fn run(ctx: &Ctx, task_id: &str) -> Result<RunOutcome, RunnerError> {
    let task = ctx
        .tasks
        .get(task_id)
        .ok_or_else(|| RunnerError::NotFound(task_id.to_string()))?;
    if task.status == TaskStatus::Done {
        return Err(RunnerError::InvalidStatus(format!(
            "task {task_id} is done; reopen before re-running (status: {:?})",
            task.status
        )));
    }
    // Open and InProgress both run: InProgress marks a running/resumed task
    // on the board while the worker session is live (F3).
    if !graph::is_ready(&ctx.tasks, task_id) {
        return Err(RunnerError::Blocked(task_id.to_string()));
    }

    // TaskContext: create `<data_dir>/tasks/<id>/` and stage brief.md before
    // the worker starts. Failure → Blocked so the run never starts with a
    // half-baked context.
    let task_dir = prep_task_context(ctx, task_id).map_err(RunnerError::Blocked)?;
    let brief_path = task_dir.join("brief.md");

    let brief = assemble_brief(task, ctx);
    std::fs::write(&brief_path, &brief)
        .map_err(|e| RunnerError::Blocked(format!("write brief {}: {e}", brief_path.display())))?;

    // F1: stage structured prompt.json alongside brief.md so the agent call
    // is observable — prompt.json (input) + events.jsonl (process) are
    // written here; result.json (output) is written by the CLI caller.
    let prompt = serde_json::json!({
        "task_id": task.id,
        "title": task.title,
        "description": task.description,
        "acceptance": task.acceptance,
        "materials": task_dir.display().to_string(),
        "deps": task.deps.iter().map(|dep| {
            let (title, status) = ctx.tasks.get(dep)
                .map(|d| (d.title.clone(), format!("{:?}", d.status)))
                .unwrap_or_else(|| ("(missing from store)".into(), "missing".into()));
            serde_json::json!({ "id": dep, "title": title, "status": status })
        }).collect::<Vec<_>>(),
        "brief": brief,
    });
    let prompt_path = task_dir.join("prompt.json");
    std::fs::write(&prompt_path, serde_json::to_string_pretty(&prompt).unwrap()).map_err(|e| {
        RunnerError::Blocked(format!("write prompt {}: {e}", prompt_path.display()))
    })?;

    // Event log: fresh file per run (truncate any prior run's log).
    let events_path = task_dir.join("events.jsonl");
    let mut events_log = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&events_path)
        .map_err(|e| {
            RunnerError::Blocked(format!("open events log {}: {e}", events_path.display()))
        })?;

    let mut worker = Worker::new(ctx.omp_path.to_str().unwrap_or("omp"))
        .map_err(|e| RunnerError::Blocked(format!("worker spawn: {e}")))?;

    // F3 liveness marker for `oi abort`: `<run-pid> <omp-pid>`. Both are
    // signalled so the worker tree dies even without job control grouping
    // the runner into its own process group.
    let pid_path = task_dir.join("run.pid");
    std::fs::write(
        &pid_path,
        format!("{} {}", std::process::id(), worker.child_pid()),
    )
    .map_err(|e| RunnerError::Blocked(format!("write run.pid: {e}")))?;
    let _pid_guard = RunPidGuard(pid_path);

    let mut events_seen = 0usize;
    let mut last_text = String::new();

    let prompt_result = worker.prompt(&brief);
    if let Err(e) = prompt_result {
        let _ = worker.abort();
        return Ok(RunOutcome {
            status: RunStatus::Failed,
            summary: format!("prompt failed: {e}"),
            events_seen,
        });
    }

    loop {
        match worker.read_event() {
            Ok(Some(event)) => {
                events_seen += 1;
                // F1: append every event to events.jsonl. Best-effort — a
                // failing diagnostic log must not fail the run.
                match serde_json::to_string(&event) {
                    Ok(line) => {
                        if let Err(e) = writeln!(events_log, "{line}") {
                            eprintln!("warning: write {}: {e}", events_path.display());
                        }
                    }
                    Err(e) => eprintln!("warning: serialize event: {e}"),
                }
                if let WorkerEvent::Message { text } = event {
                    println!("[worker] {text}");
                    if !text.is_empty() {
                        last_text = text;
                    }
                }
                // F3: poll the steer inbox between events so an external
                // `oi steer` can drive the live worker. Abort uses a
                // process-group signal (`run.pid` + kill -TERM -<pid>) so it
                // works even while the model is silent — the poll can't.
                if let Err(e) = poll_steer(&task_dir, &mut worker) {
                    eprintln!("warning: steer poll: {e}");
                }
            }
            Ok(None) => {
                // A response frame was delivered (prompt accepted or a
                // transport-level marker), and omp currently has no further
                // events queued. Treat as terminal idle and exit cleanly.
                let _ = worker.abort();
                return Ok(RunOutcome {
                    status: RunStatus::Done,
                    summary: if last_text.is_empty() {
                        "no final message".into()
                    } else {
                        last_text
                    },
                    events_seen,
                });
            }
            Err(e) => {
                let _ = worker.abort();
                return Ok(RunOutcome {
                    status: RunStatus::Failed,
                    summary: format!("read_event error: {e}"),
                    events_seen,
                });
            }
        }
    }
}

/// RAII: remove `<task_dir>/run.pid` on drop so every run exit path cleans
/// up the liveness marker consumed by `oi abort`.
struct RunPidGuard(PathBuf);

impl Drop for RunPidGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Poll the steer inbox: `steer-cmd.txt` is forwarded to the worker and
/// removed.
///
/// ponytail: polling happens only at event boundaries — a steer issued
/// during a long silent thinking stretch is delivered at the next event.
/// Replace with a dedicated control channel if interactive latency matters.
fn poll_steer(task_dir: &std::path::Path, worker: &mut Worker) -> Result<(), String> {
    let steer_path = task_dir.join("steer-cmd.txt");
    if steer_path.exists() {
        let text =
            std::fs::read_to_string(&steer_path).map_err(|e| format!("read steer inbox: {e}"))?;
        let _ = std::fs::remove_file(&steer_path);
        let msg = text.trim();
        if !msg.is_empty() {
            worker
                .steer(msg)
                .map_err(|e| format!("steer forward: {e}"))?;
        }
    }
    Ok(())
}

/// Assemble the MVP brief: description + acceptance + materials path, plus a
/// `deps_results` section when the task has completed dependencies (M3 §4).
fn assemble_brief(task: &Task, ctx: &Ctx) -> String {
    let mut brief = String::new();
    brief.push_str(&format!("## Task: {}\n{}", task.id, task.description));
    if !task.acceptance.is_empty() {
        brief.push_str(&format!("\n## Acceptance criteria\n{}", task.acceptance));
    }
    brief.push_str(&format!(
        "\n## Task materials\n{}",
        ctx.data_dir.join("tasks").join(&task.id).display()
    ));
    if !task.deps.is_empty() {
        brief.push_str("\n## Dependencies\n");
        for dep in &task.deps {
            let summary = ctx
                .tasks
                .get(dep)
                .map(|d| format!("- {dep}: {} — status {:?}", d.title, d.status))
                .unwrap_or_else(|| format!("- {dep}: (missing from store)"));
            brief.push_str(&summary);
            brief.push('\n');
        }
    }
    brief
}

/// Create `<data_dir>/tasks/<task_id>/` if needed. Returns the directory path.
///
/// Idempotent — repeated runs of the same task reuse the same directory.
fn prep_task_context(ctx: &Ctx, task_id: &str) -> Result<PathBuf, String> {
    let dir = ctx.data_dir.join("tasks").join(task_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph;

    fn mk_task(id: &str, deps: Vec<String>, status: TaskStatus) -> Task {
        Task {
            id: id.to_string(),
            title: format!("task {id}"),
            kind: crate::task::TaskKind::Task,
            status,
            priority: 2,
            parent: None,
            deps,
            description: format!("desc {id}"),
            acceptance: String::new(),
            created_at: "2026-01-01".into(),
            updated_at: "2026-01-01".into(),
        }
    }
    #[test]
    fn run_stages_artifacts_before_worker_spawn() {
        // /bin/true is not an omp RPC server → Worker::new fails, but the
        // staged artifacts (brief.md + prompt.json + events.jsonl) must
        // already exist when it does (F1 observable input/process/output).
        let task = mk_task("artifact-task", vec![], TaskStatus::Open);
        let (ctx, _tmp) = ctx_with_tmp(vec![task.clone()]);
        let r = run(&ctx, "artifact-task");
        assert!(matches!(r, Err(RunnerError::Blocked(s)) if s.contains("worker spawn")));
        let dir = ctx.data_dir.join("tasks/artifact-task");
        assert!(dir.join("brief.md").exists());
        assert!(dir.join("prompt.json").exists());
        assert!(dir.join("events.jsonl").exists());
        let prompt: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("prompt.json")).unwrap())
                .unwrap();
        assert_eq!(prompt["task_id"], "artifact-task");
        assert_eq!(prompt["deps"], serde_json::json!([]));
        assert!(
            prompt["brief"]
                .as_str()
                .unwrap()
                .contains("desc artifact-task")
        );
    }

    #[test]
    fn prompt_json_lists_completed_deps_with_status() {
        let dep = Task {
            id: "dep-1".into(),
            title: "scaffold".into(),
            kind: crate::task::TaskKind::Task,
            status: TaskStatus::Done,
            priority: 2,
            parent: None,
            deps: vec![],
            description: String::new(),
            acceptance: String::new(),
            created_at: "2026-01-01".into(),
            updated_at: "2026-01-01".into(),
        };
        let task = mk_task("feat-x", vec!["dep-1".into()], TaskStatus::Open);
        let (ctx, _tmp) = ctx_with_tmp(vec![dep, task.clone()]);
        let _ = run(&ctx, "feat-x");
        let dir = ctx.data_dir.join("tasks/feat-x");
        let prompt: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("prompt.json")).unwrap())
                .unwrap();
        let deps = prompt["deps"].as_array().unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0]["id"], "dep-1");
        assert_eq!(deps[0]["status"], "Done");
        assert_eq!(deps[0]["title"], "scaffold");
    }

    fn ctx(tasks: Vec<Task>, omp_path: &str) -> Ctx {
        Ctx {
            omp_path: omp_path.into(),
            tasks: tasks.into_iter().map(|t| (t.id.clone(), t)).collect(),
            data_dir: "/tmp/omenic-test".into(),
        }
    }

    #[test]
    fn run_delegates_deps_gate() {
        // dep-todo is still open → blocked
        let tasks = vec![
            mk_task("dep-todo", vec![], TaskStatus::Open),
            mk_task("mine", vec!["dep-todo".into()], TaskStatus::Open),
        ];
        let r = run(&ctx(tasks, "/bin/true"), "mine");
        assert!(matches!(r, Err(RunnerError::Blocked(s)) if s == "mine"));
    }

    #[test]
    fn run_missing_task() {
        let r = run(&ctx(vec![], "/bin/true"), "ghost");
        assert!(matches!(r, Err(RunnerError::NotFound(s)) if s == "ghost"));
    }

    fn ctx_with_tmp(tasks: Vec<Task>) -> (Ctx, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = Ctx {
            omp_path: "/bin/true".into(),
            tasks: tasks.into_iter().map(|t| (t.id.clone(), t)).collect(),
            data_dir: tmp.path().to_path_buf(),
        };
        (ctx, tmp)
    }

    #[test]
    fn prep_task_context_creates_dirs_idempotent() {
        let (ctx, _tmp) = ctx_with_tmp(vec![]);
        let dir = prep_task_context(&ctx, "my-task").unwrap();
        assert!(dir.exists());
        assert!(dir.ends_with("tasks/my-task"));
        // Second call reuses — no error.
        let dir2 = prep_task_context(&ctx, "my-task").unwrap();
        assert_eq!(dir, dir2);
    }

    #[test]
    fn brief_lists_completed_deps_when_present() {
        let dep = Task {
            id: "dep-1".into(),
            title: "scaffold".into(),
            kind: crate::task::TaskKind::Task,
            status: TaskStatus::Done,
            priority: 2,
            parent: None,
            deps: vec![],
            description: String::new(),
            acceptance: String::new(),
            created_at: "2026-01-01".into(),
            updated_at: "2026-01-01".into(),
        };
        let task = mk_task("feat-x", vec!["dep-1".into()], TaskStatus::Open);
        let (ctx, _tmp) = ctx_with_tmp(vec![dep, task.clone()]);
        let brief = assemble_brief(&task, &ctx);
        assert!(brief.contains("## Task materials"));
        assert!(brief.contains("## Dependencies"));
        assert!(brief.contains("- dep-1: scaffold — status Done"));
    }

    #[test]
    fn brief_without_deps_omits_dependency_section() {
        let task = mk_task("lonely", vec![], TaskStatus::Open);
        let (ctx, _tmp) = ctx_with_tmp(vec![task.clone()]);
        let brief = assemble_brief(&task, &ctx);
        assert!(!brief.contains("## Dependencies"));
    }

    #[test]
    fn run_delegates_status_guard() {
        // Done tasks are rejected (would re-run a finished step)…
        let done = mk_task("done-already", vec![], TaskStatus::Done);
        let r = run(&ctx(vec![done], "/bin/true"), "done-already");
        assert!(matches!(r, Err(RunnerError::InvalidStatus(s)) if s.contains("done")));

        // …but InProgress (F3 running marker) is allowed so an aborted or
        // resumed task can re-run; /bin/true is not omp → fails at spawn.
        let in_progress = mk_task("in-progress", vec![], TaskStatus::InProgress);
        let (ctx, _tmp) = ctx_with_tmp(vec![in_progress]);
        let r = run(&ctx, "in-progress");
        assert!(matches!(r, Err(RunnerError::Blocked(s)) if s.contains("worker spawn")));
    }

    #[test]
    fn assemble_brief_includes_acceptance_and_materials() {
        let task = mk_task("auth-flow", vec![], TaskStatus::Open);
        let ctx = ctx(vec![task.clone()], "/bin/true");
        let brief = assemble_brief(&task, &ctx);
        assert!(brief.contains("auth-flow"));
        // No acceptance string → no acceptance heading.
        assert!(!brief.contains("## Acceptance criteria"));
        assert!(brief.contains("/tmp/omenic-test/tasks/auth-flow"));

        let task2 = Task {
            acceptance: "tests pass; swagger docs".into(),
            ..task
        };
        let brief2 = assemble_brief(&task2, &ctx);
        assert!(brief2.contains("## Acceptance criteria"));
        assert!(brief2.contains("tests pass; swagger docs"));
    }

    #[test]
    fn is_ready_rule_blocks_until_deps_done() {
        let tasks: Vec<Task> = vec![
            mk_task("a", vec![], TaskStatus::Done),
            mk_task("b", vec!["a".into()], TaskStatus::Open),
            mk_task("c", vec!["a".into(), "b".into()], TaskStatus::Open),
        ];
        let map: HashMap<String, Task> = tasks.iter().map(|t| (t.id.clone(), t.clone())).collect();
        assert!(graph::is_ready(&map, "b"));
        assert!(!graph::is_ready(&map, "c"));
    }
}
