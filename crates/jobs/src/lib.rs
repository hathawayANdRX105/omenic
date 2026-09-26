//! Background job registry for omenic.
//!
//! A *job* is work that must outlive the tool call which started it — a long
//! build, a training run, a server. The agent's tool call returns as soon as
//! the job is registered, and later tool calls poll (`list`), collect
//! (`wait`) or stop (`kill`) it.
//!
//! # Why jobs run on OS threads
//!
//! `orbit` calls `Tool::execute` synchronously on the engine thread (see the
//! tool-call loop in `crates/agent/orbit/src/lib.rs`). There is no async event
//! loop to park work on: the single `tokio` runtime in the workspace is a
//! private current-thread runtime owned by `SessionDb`. So a job is driven by
//! a `std::thread` it spawns, and the registry synchronises on plain `Mutex` +
//! `Condvar`. This mirrors how the MCP client (B1) was built — `std::thread` +
//! `mpsc` + `Mutex`, no new runtime.
//!
//! # Lifecycle
//!
//! ```text
//!            start()
//!              |
//!              v
//!          [running] --complete--> [completed]
//!              |  \-----fail-----> [failed]
//!              \--------kill()----> [killed]
//! ```
//!
//! `completed` / `failed` / `killed` are terminal; a terminal job keeps its
//! record (and captured output) so a `wait` after the fact still returns it.

use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use serde::{Deserialize, Serialize};

/// Lock a shared-state mutex, recovering from poisoning.
///
/// A panic elsewhere must not turn the whole registry into a landmine: the
/// mutated state here is a table of job records, and the code that touches it
/// only reads and writes that table. `crates/agent/mcp` and
/// `crates/infra/daemon` take the same view, so a poisoned lock degrades to
/// "keep going" rather than "panic on every later call".
fn state_guard<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// -----------------------------------------------------------------------------
// Identifiers
// -----------------------------------------------------------------------------

/// Opaque job identifier.
///
/// Minted by the registry as `job-<n>` so it is stable across `list` calls and
/// safe to hand to the model. Not a UUID: nothing here needs global
/// uniqueness, and a counter keeps the id short enough to read in a
/// transcript.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JobId(String);

impl JobId {
    /// Wrap an existing string as a job id.
    ///
    /// Used by `kill`/`wait` callers that round-tripped an id through JSON.
    /// The registry never accepts a caller-chosen id at `start` — ids are
    /// minted internally so two callers cannot collide.
    pub fn new(id: impl Into<String>) -> Self {
        JobId(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for JobId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// -----------------------------------------------------------------------------
// State
// -----------------------------------------------------------------------------

/// State of one job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Running,
    Completed,
    Failed,
    Killed,
}

impl JobState {
    /// Whether this state is terminal (no further transition is possible).
    pub fn is_terminal(self) -> bool {
        !matches!(self, JobState::Running)
    }
}

impl Display for JobState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            JobState::Running => "running",
            JobState::Completed => "completed",
            JobState::Failed => "failed",
            JobState::Killed => "killed",
        };
        f.write_str(s)
    }
}

/// What a finished job produced.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobOutput {
    /// Everything the job wrote to its primary channel (stdout for a process
    /// job, the pty screen buffer for a terminal job). Empty until terminal.
    pub stdout: String,
    /// Everything the job wrote to stderr. Always empty for pty-backed jobs,
    /// which merge both streams into the pty.
    pub stderr: String,
    /// Process exit code, when the job is process-backed. `None` for a job
    /// that was killed before spawn reached an exit.
    pub exit_code: Option<i32>,
}

/// One row of [`JobRegistry::list`], without the captured output.
///
/// Kept separate from [`JobOutput`] because listing should stay cheap — a job
/// that has written megabytes must not have them cloned into a status reply.
/// `wait` is the call that returns the output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobSummary {
    pub id: JobId,
    /// Human-readable label set at `start` (e.g. the command line). Shown in
    /// `jobs_list` so the model can tell two jobs apart.
    pub label: String,
    pub state: JobState,
    /// Milliseconds since the registry was created, for ordering a listing.
    pub started_ms: u64,
    /// Present once the job reached a terminal state with a known exit.
    pub exit_code: Option<i32>,
}

// -----------------------------------------------------------------------------
// Task + errors
// -----------------------------------------------------------------------------

/// The work a job runs.
///
/// The closure receives a [`JobControl`] it should poll for cancellation. The
/// registry cannot interrupt a running closure from outside, so a task that
/// ignores `is_cancelled()` keeps running after `kill` and is reported killed
/// only once it returns. That is a deliberate trade: forcing cancellation to be
/// observable would need every task to thread a signal through, and the common
/// case (`std::process::Child::kill`) already gives real interruption.
pub type JobTask = Box<dyn FnOnce(&JobControl) -> Result<JobOutput, String> + Send + 'static>;

/// Cancellation flag handed to a running [`JobTask`].
#[derive(Debug, Clone, Default)]
pub struct JobControl {
    cancelled: Arc<std::sync::atomic::AtomicBool>,
}

impl JobControl {
    /// Whether `kill` has been called for this job.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// Request cancellation. Called by the registry, exposed for tests.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// The cancellation flag, for a task that needs to hand it to helpers.
    pub fn flag(&self) -> Arc<std::sync::atomic::AtomicBool> {
        Arc::clone(&self.cancelled)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum JobError {
    #[error("unknown job: {0}")]
    Unknown(JobId),
    /// The job had not reached a terminal state when a non-blocking caller
    /// needed it to. Produced by `wait` on timeout; the job is untouched and
    /// still waitable.
    #[error("job {0} is still running")]
    StillRunning(JobId),
    /// The registry was shut down, or its running-job ceiling is reached.
    #[error("job registry refused the work: {0}")]
    Refused(String),
}

// -----------------------------------------------------------------------------
// Trait
// -----------------------------------------------------------------------------

/// Registry of background jobs.
///
/// Every method takes `&self`: the registry synchronises internally so one
/// `Arc<dyn JobRegistry>` can be shared by the daemon, the worker and every
/// tool clone. `&mut self` would make that impossible, which is why this
/// deviates from the async-shaped sketch in `todo/archive/dsh-backlog-detail.md` (that
/// sketch also assumed a tokio runtime this workspace does not have).
pub trait JobRegistry: Send + Sync {
    /// Register `task` under `label` and return its id immediately.
    ///
    /// The task starts on its own thread; `start` does not block on it.
    fn start(&self, label: impl Into<String>, task: JobTask) -> Result<JobId, JobError>;

    /// All known jobs, newest first. Includes terminal jobs.
    fn list(&self) -> Vec<JobSummary>;

    /// One job's summary, if known.
    fn status(&self, id: &JobId) -> Result<JobSummary, JobError>;

    /// Request cancellation.
    ///
    /// Returns the state the job was in *before* the call, so a caller can tell
    /// "I stopped it" from "it had already finished". Killing a terminal job is
    /// not an error — it is a no-op that reports the existing state.
    fn kill(&self, id: &JobId) -> Result<JobState, JobError>;

    /// Block until `id` reaches a terminal state (or `timeout_ms` elapses),
    /// then return its output.
    ///
    /// `None` waits indefinitely. `Some(0)` is a non-blocking poll. Either way
    /// a timeout returns [`JobError::StillRunning`] rather than lying with a
    /// partial result.
    fn wait(&self, id: &JobId, timeout_ms: Option<u64>) -> Result<JobOutput, JobError>;

    /// Forget a terminal job. Returns whether it was present.
    ///
    /// Running jobs are never removed — dropping the record would leak the
    /// thread's join handle and make the job unkillable.
    fn remove(&self, id: &JobId) -> bool;
}

// -----------------------------------------------------------------------------
// In-memory implementation
// -----------------------------------------------------------------------------

/// One job's full record.
struct JobRecord {
    label: String,
    state: JobState,
    started_ms: u64,
    output: JobOutput,
    control: JobControl,
    /// Kept so a future `shutdown` can join, and so the handle is not leaked.
    /// Cleared the moment the job turns terminal.
    join: Option<std::thread::JoinHandle<()>>,
}

impl JobRecord {
    fn summary(&self, id: &JobId) -> JobSummary {
        JobSummary {
            id: id.clone(),
            label: self.label.clone(),
            state: self.state,
            started_ms: self.started_ms,
            exit_code: self.output.exit_code,
        }
    }
}

/// Mutable registry state, guarded by one mutex.
///
/// A single lock is correct here: every critical section is a few field reads
/// or writes with no I/O and no call back into the registry, so contention is
/// irrelevant and one lock keeps the state machine obviously consistent.
struct RegistryState {
    jobs: HashMap<JobId, JobRecord>,
    /// Insertion counter for minting ids and breaking listing ties.
    seq: u64,
    /// Clock origin, so `started_ms` is comparable within a process.
    epoch: std::time::Instant,
    shutdown: bool,
}

impl RegistryState {
    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }
}

impl Drop for RegistryState {
    fn drop(&mut self) {
        // Detach every join handle explicitly. Dropping a `JoinHandle` already
        // detaches, but doing it here documents that the registry never joins:
        // shutdown must not block on a task that ignores cancellation.
        for (_, rec) in self.jobs.drain() {
            drop(rec.join);
        }
    }
}

/// The parts a job thread needs. Cloned into each spawned closure so the
/// thread does not borrow the registry.
struct Shared {
    state: Mutex<RegistryState>,
    changed: Condvar,
    done_hook: Mutex<Option<JobDoneHook>>,
}

/// Callback fired when a job reaches a terminal state (grok `onJobDone`).
/// Takes the summary by reference; the registry is not borrowed and the
/// state lock is not held when it runs, so a hook may call back into the
/// registry.
pub type JobDoneHook = std::sync::Arc<dyn Fn(&JobSummary) + Send + Sync>;

impl Shared {
    /// Mark `id` terminal and wake every `wait` caller.
    ///
    /// Always called with `state` locked. The notify happens after the
    /// mutation, so a waiter that observes the new state cannot miss the
    /// wakeup.
    fn finish_locked(
        state: &mut RegistryState,
        changed: &Condvar,
        id: &JobId,
        record_state: JobState,
        output: JobOutput,
    ) {
        if let Some(rec) = state.jobs.get_mut(id) {
            // A killed job stays killed: the kill was the observed outcome,
            // and reporting `completed` would imply the work ran to a real
            // conclusion.
            if rec.state != JobState::Killed {
                rec.state = record_state;
            }
            rec.output = output;
            // The thread is returning; drop the handle rather than keep a
            // stale one that could never be joined from the registry lock.
            rec.join = None;
        }
        changed.notify_all();
    }
}

/// In-memory [`JobRegistry`].
///
/// Jobs live only as long as the process: there is no persistence, matching
/// dsh's `LocalJobRegistry`. A daemon restart drops every job.
pub struct LocalJobRegistry {
    shared: Arc<Shared>,
    /// Ceiling on simultaneously *running* jobs. Stored separately so
    /// `set_max_running` does not need `&mut self`.
    max_running: AtomicU64,
}

impl Default for LocalJobRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalJobRegistry {
    /// Default cap on simultaneously running jobs.
    ///
    /// A runaway model can call `jobs_start` in a loop; without a ceiling each
    /// call costs an OS thread. Terminal jobs do not count, so removing a
    /// finished job always makes room.
    pub const MAX_RUNNING: usize = 64;

    pub fn new() -> Self {
        LocalJobRegistry {
            shared: Arc::new(Shared {
                state: Mutex::new(RegistryState {
                    jobs: HashMap::new(),
                    seq: 0,
                    epoch: std::time::Instant::now(),
                    shutdown: false,
                }),
                changed: Condvar::new(),
                done_hook: Mutex::new(None),
            }),
            max_running: AtomicU64::new(Self::MAX_RUNNING as u64),
        }
    }

    /// Count of jobs currently in `Running`.
    fn running_count(state: &RegistryState) -> usize {
        state
            .jobs
            .values()
            .filter(|r| r.state == JobState::Running)
            .count()
    }

    /// Stop accepting work and cancel every running job.
    ///
    /// Called on daemon shutdown. Jobs are cancelled but never joined: a task
    /// that ignores cancellation would otherwise stall shutdown, and process
    /// exit reclaims the threads regardless.
    pub fn shutdown(&self) {
        let mut state = state_guard(&self.shared.state);
        state.shutdown = true;
        for rec in state.jobs.values_mut() {
            if rec.state == JobState::Running {
                rec.control.cancel();
                rec.state = JobState::Killed;
            }
        }
        state.jobs.clear();
        self.shared.changed.notify_all();
    }

    /// Register a hook fired once per job that reaches a terminal state.
    ///
    /// The hook runs on the job's own thread *after* the registry lock is
    /// released, so a slow consumer cannot stall the registry and a hook may
    /// re-enter it. Re-registering replaces the previous hook — one consumer
    /// (the daemon's aside channel) is the expected shape.
    pub fn on_job_done(&self, hook: JobDoneHook) {
        *self
            .shared
            .done_hook
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(hook);
    }

    /// Override the running-job ceiling.
    ///
    /// Gated on a compile-time test attribute rather than this being available
    /// only to unit tests: integration tests under `tests/` link the crate as a
    /// dependency, so a test-attribute-gated item would be invisible to them.
    pub fn set_max_running(&self, max: usize) {
        self.max_running.store(max as u64, Ordering::Relaxed);
    }
}

impl JobRegistry for LocalJobRegistry {
    fn start(&self, label: impl Into<String>, task: JobTask) -> Result<JobId, JobError> {
        let label = label.into();
        let control = JobControl::default();
        let id;
        {
            let mut state = state_guard(&self.shared.state);
            if state.shutdown {
                return Err(JobError::Refused("registry is shut down".into()));
            }
            let cap = self.max_running.load(Ordering::Relaxed) as usize;
            let running = Self::running_count(&state);
            if running >= cap {
                return Err(JobError::Refused(format!(
                    "{running} jobs already running (ceiling {cap}); wait for or remove one"
                )));
            }
            state.seq += 1;
            id = JobId(format!("job-{}", state.seq));
            let started_ms = state.now_ms();
            state.jobs.insert(
                id.clone(),
                JobRecord {
                    label,
                    state: JobState::Running,
                    started_ms,
                    output: JobOutput::default(),
                    control: control.clone(),
                    join: None,
                },
            );
        }

        let shared = Arc::clone(&self.shared);
        let id_for_thread = id.clone();
        let task_control = control.clone();
        let handle = std::thread::Builder::new()
            .name(format!("omenic-job-{}", id.as_str()))
            .spawn(move || {
                let outcome = task(&task_control);
                let (record_state, output) = match outcome {
                    Ok(out) => (
                        // A task that cooperated with cancellation is reported
                        // killed even if it returned Ok, so `kill` stays
                        // observable.
                        if task_control.is_cancelled() {
                            JobState::Killed
                        } else {
                            JobState::Completed
                        },
                        out,
                    ),
                    Err(msg) => (
                        JobState::Failed,
                        JobOutput {
                            stderr: msg,
                            ..JobOutput::default()
                        },
                    ),
                };
                let mut state = state_guard(&shared.state);
                Shared::finish_locked(
                    &mut state,
                    &shared.changed,
                    &id_for_thread,
                    record_state,
                    output,
                );
                let summary = state
                    .jobs
                    .get(&id_for_thread)
                    .map(|rec| rec.summary(&id_for_thread));
                drop(state);
                // Outside the lock on purpose: the hook is consumer code.
                if let Some(hook) = shared
                    .done_hook
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone()
                {
                    if let Some(summary) = summary.as_ref() {
                        hook(summary);
                    }
                }
            })
            .expect("job thread spawn");

        {
            let mut state = state_guard(&self.shared.state);
            match state.jobs.get_mut(&id) {
                // Still running: keep the handle.
                Some(rec) if rec.state == JobState::Running => rec.join = Some(handle),
                // Finished between insert and now — `finish_locked` already
                // cleared the slot; detach rather than resurrect.
                _ => drop(handle),
            }
        }
        Ok(id)
    }

    fn list(&self) -> Vec<JobSummary> {
        let state = state_guard(&self.shared.state);
        let mut rows: Vec<JobSummary> =
            state.jobs.iter().map(|(id, rec)| rec.summary(id)).collect();
        // Newest first. `started_ms` ties break on the id, which carries the
        // insertion counter, so the order is total — HashMap iteration order
        // would otherwise leak into the listing.
        rows.sort_by(|a, b| {
            b.started_ms
                .cmp(&a.started_ms)
                .then_with(|| b.id.as_str().cmp(a.id.as_str()))
        });
        rows
    }

    fn status(&self, id: &JobId) -> Result<JobSummary, JobError> {
        let state = state_guard(&self.shared.state);
        state
            .jobs
            .get(id)
            .map(|rec| rec.summary(id))
            .ok_or_else(|| JobError::Unknown(id.clone()))
    }

    fn kill(&self, id: &JobId) -> Result<JobState, JobError> {
        let mut state = state_guard(&self.shared.state);
        let rec = state
            .jobs
            .get_mut(id)
            .ok_or_else(|| JobError::Unknown(id.clone()))?;
        let previous = rec.state;
        if previous.is_terminal() {
            return Ok(previous);
        }
        rec.control.cancel();
        rec.state = JobState::Killed;
        self.shared.changed.notify_all();
        Ok(previous)
    }

    fn wait(&self, id: &JobId, timeout_ms: Option<u64>) -> Result<JobOutput, JobError> {
        let mut state = state_guard(&self.shared.state);
        if !state.jobs.contains_key(id) {
            return Err(JobError::Unknown(id.clone()));
        }

        let deadline =
            timeout_ms.map(|ms| state.epoch.elapsed() + std::time::Duration::from_millis(ms));

        loop {
            let rec = state.jobs.get(id).expect("presence checked above");
            if rec.state.is_terminal() {
                return Ok(rec.output.clone());
            }
            let Some(deadline) = deadline else {
                // A poisoned wait mutex means some other holder panicked while
                // the record was mid-update. The table is still readable, so
                // recover the guard and re-check the state rather than
                // propagating a panic into the caller's loop.
                state = match self.shared.changed.wait(state) {
                    Ok(next) => next,
                    Err(poisoned) => poisoned.into_inner(),
                };
                continue;
            };
            let remaining = deadline.saturating_sub(state.epoch.elapsed());
            if remaining.is_zero() {
                return Err(JobError::StillRunning(id.clone()));
            }
            let (next, timeout) = match self.shared.changed.wait_timeout(state, remaining) {
                Ok(pair) => pair,
                Err(poisoned) => poisoned.into_inner(),
            };
            state = next;
            if timeout.timed_out() {
                // The job may have finished in the window between the timeout
                // firing and re-acquiring the lock — re-check before reporting.
                if state.jobs.get(id).is_some_and(|r| r.state.is_terminal()) {
                    continue;
                }
                return Err(JobError::StillRunning(id.clone()));
            }
        }
    }

    fn remove(&self, id: &JobId) -> bool {
        let mut state = state_guard(&self.shared.state);
        match state.jobs.get(id) {
            Some(rec) if rec.state.is_terminal() => {
                state.jobs.remove(id);
                true
            }
            _ => false,
        }
    }
}
