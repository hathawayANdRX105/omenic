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

use llm;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use crate::rpc::worker::{Worker, WorkerEvent};
use store::graph;
use store::{Task, TaskStatus};

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
    /// Attempt budget exhausted (#47); the task stays Failed until the
    /// counter is reset explicitly.
    RetryLimit {
        id: String,
        attempts: u32,
    },
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
            RunnerError::RetryLimit { id, attempts } => write!(
                f,
                "retry limit exceeded for {id} ({attempts} failed attempts); \
                 reset with `oi task update {id} --attempts 0`"
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
    /// External MCP servers that supply additional tool definitions.
    pub mcp_servers: Vec<config::McpServerConfig>,
}

/// Failed-attempt budget per task before `run` refuses further retries
/// (#47). ponytail: a constant until someone needs per-workspace tuning —
/// promote to Config then.
pub const MAX_ATTEMPTS: u32 = 3;

/// True iff `<task_dir>/run.pid` exists and its recorded runner pid is
/// still alive. An InProgress task without a live runner is an orphan:
/// the runner crashed or was SIGKILLed before it could flip status (#47).
///
/// ponytail: /proc lookup is Linux-only; switch to `libc::kill(pid, 0)`
/// if another OS ever matters. Pid reuse can fool this check — acceptable
/// for an MVP liveness hint.
pub fn runner_alive(task_dir: &std::path::Path) -> bool {
    let Ok(content) = std::fs::read_to_string(task_dir.join("run.pid")) else {
        return false;
    };
    let Some(pid) = content
        .split_whitespace()
        .next()
        .and_then(|p| p.parse::<u32>().ok())
    else {
        return false;
    };
    std::path::Path::new("/proc").join(pid.to_string()).exists()
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
    // Open, InProgress (resume of an orphaned task) and Failed (retry) all
    // run; only Done is terminal (#47).
    if !graph::is_ready(&ctx.tasks, task_id) {
        return Err(RunnerError::Blocked(task_id.to_string()));
    }
    // Retry gate AFTER the deps gate so an exhausted budget can never
    // bypass the blocked check (#47).
    if task.attempts >= MAX_ATTEMPTS {
        return Err(RunnerError::RetryLimit {
            id: task_id.to_string(),
            attempts: task.attempts,
        });
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

    // Event log: append-only across runs so re-runs never destroy prior
    // evidence; a `run_start` record marks each run's boundary (#48).
    let events_path = task_dir.join("events.jsonl");
    let mut events_log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&events_path)
        .map_err(|e| {
            RunnerError::Blocked(format!("open events log {}: {e}", events_path.display()))
        })?;
    append_record(&mut events_log, &events_path, &run_start_record(task_id));

    let mut worker = Worker::new(ctx.omp_path.to_str().unwrap_or("omp"), None)
        .map_err(|e| RunnerError::Blocked(format!("worker spawn: {e}")))?;
    let abort_signal = AtomicBool::new(false);
    // MCP bring-up happens before the worker registers tools: a server with
    // `fail_on_startup_error` that fails blocks the run instead of starting
    // it with a silently missing toolset.
    let external = mcp::external_tools_from_mcp(&ctx.mcp_servers, &abort_signal)
        .map_err(|e| RunnerError::Blocked(format!("mcp: {e}")))?;
    let tool_boxes = tools::builtin_tools()
        .into_iter()
        .chain(external)
        .collect::<Vec<_>>();
    let defs: Vec<llm::ToolDef> = tool_boxes
        .iter()
        .map(|tool_box| tools::def(&**tool_box))
        .collect();
    if let Err(e) = worker.register_external_tools(defs) {
        eprintln!("warning: omp rejected external tools: {e}");
    }

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
                // F1: append every event to events.jsonl as a bounded
                // timestamped record (#48). Best-effort — a failing
                // diagnostic log must not fail the run.
                append_event(&mut events_log, &events_path, &event);
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

/// Max serialized bytes for one payload field inside an events.jsonl
/// record; larger fields degrade to a truncated prefix (#48).
const EVENT_FIELD_MAX_BYTES: usize = 1024;

/// Longest char-boundary prefix of `s` that fits in `max_bytes` — never
/// splits a multi-byte character.
fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Bounded JSON for a payload field: compact serializations within
/// `EVENT_FIELD_MAX_BYTES` pass through unchanged; larger ones become a
/// truncated string prefix and set `truncated`.
fn bounded_value(v: &serde_json::Value, truncated: &mut bool) -> serde_json::Value {
    let s = v.to_string();
    if s.len() <= EVENT_FIELD_MAX_BYTES {
        v.clone()
    } else {
        *truncated = true;
        serde_json::Value::String(format!("{}…", truncate_utf8(&s, EVENT_FIELD_MAX_BYTES)))
    }
}

/// One events.jsonl record: timestamp + event type + bounded summary.
/// Never stores unbounded raw RPC payloads (#48).
fn event_record(event: &WorkerEvent) -> serde_json::Value {
    let mut truncated = false;
    let mut rec = match event {
        WorkerEvent::AgentStart => serde_json::json!({ "event": "agent_start" }),
        WorkerEvent::AgentEnd { stop_reason } => serde_json::json!({
            "event": "agent_end",
            "stop_reason": stop_reason
        }),
        WorkerEvent::Message { text } => {
            if text.len() > EVENT_FIELD_MAX_BYTES {
                truncated = true;
            }
            serde_json::json!({ "event": "message", "text": truncate_utf8(text, EVENT_FIELD_MAX_BYTES) })
        }
        WorkerEvent::Reasoning { delta } => {
            if delta.len() > EVENT_FIELD_MAX_BYTES {
                truncated = true;
            }
            serde_json::json!({
                "event": "reasoning",
                "delta": truncate_utf8(delta, EVENT_FIELD_MAX_BYTES),
            })
        }
        WorkerEvent::ToolExecutionStart { name, input } => serde_json::json!({
            "event": "tool_execution_start",
            "name": name,
            "input": bounded_value(input, &mut truncated),
        }),
        WorkerEvent::ToolExecutionEnd { name, result } => serde_json::json!({
            "event": "tool_execution_end",
            "name": name,
            "result": match result {
                Some(v) => bounded_value(v, &mut truncated),
                None => serde_json::Value::Null,
            },
        }),
        WorkerEvent::Error { error } => {
            serde_json::json!({ "event": "error", "error": error })
        }
        WorkerEvent::Unknown(raw) => {
            serde_json::json!({ "event": "unknown", "raw": bounded_value(raw, &mut truncated) })
        }
    };
    if truncated {
        rec["truncated"] = serde_json::json!(true);
    }
    rec["ts"] = serde_json::json!(store::now_iso());
    rec
}

/// Run-boundary record written when the event log opens, so evidence from
/// multiple runs stays ordered and separable in one append-only file (#48).
fn run_start_record(task_id: &str) -> serde_json::Value {
    serde_json::json!({
        "ts": store::now_iso(),
        "event": "run_start",
        "task_id": task_id,
        "pid": std::process::id(),
    })
}

/// Serialize one record and append it as a single line. Best-effort:
/// failures warn on stderr and never fail the run, so event-log trouble
/// cannot break result.json consistency (#48).
fn append_record<W: Write>(log: &mut W, path: &std::path::Path, record: &serde_json::Value) {
    match serde_json::to_string(record) {
        Ok(line) => {
            if let Err(e) = writeln!(log, "{line}") {
                eprintln!("warning: write {}: {e}", path.display());
            }
        }
        Err(e) => eprintln!("warning: serialize event record: {e}"),
    }
}

/// Append one worker event to the log as a bounded timestamped record.
fn append_event<W: Write>(log: &mut W, path: &std::path::Path, event: &WorkerEvent) {
    append_record(log, path, &event_record(event));
}
