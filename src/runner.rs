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
}

impl std::fmt::Display for RunnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunnerError::NotFound(id) => write!(f, "task not found: {id}"),
            RunnerError::Blocked(id) => write!(
                f,
                "deps not ready for {id}; blocked until predecessors complete"
            ),
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
    if task.status != TaskStatus::Done {
        // We don't check status here — the caller flips it after we're done.
        // Ready gate is pure deps.
    }
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

    let mut worker = Worker::new(ctx.omp_path.to_str().unwrap_or("omp"))
        .map_err(|e| RunnerError::Blocked(format!("worker spawn: {e}")))?;

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
                if let WorkerEvent::Message { text } = event {
                    println!("[worker] {text}");
                    if !text.is_empty() {
                        last_text = text;
                    }
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
            parent: None,
            deps,
            description: format!("desc {id}"),
            acceptance: String::new(),
            created_at: "2026-01-01".into(),
            updated_at: "2026-01-01".into(),
        }
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
