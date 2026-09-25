//! Background-job tools (`jobs_start`/`wait`/`list`/`kill`), moved out of
//! `jobs_terminal.rs` into their own module. Shared helpers stay in the parent
//! and are reached via `super::*`.

use super::*;
use serde_json::Value;

use crate::{Tool, ToolError};
use jobs::{JobError, JobId, JobRegistry, JobState, LocalJobRegistry};

// -----------------------------------------------------------------------------
// jobs_start
// -----------------------------------------------------------------------------

pub struct JobsStart {
    reg: Arc<LocalJobRegistry>,
}

impl JobsStart {
    pub fn new(reg: Arc<LocalJobRegistry>) -> Self {
        Self { reg }
    }
}

impl Tool for JobsStart {
    fn name(&self) -> &str {
        "jobs_start"
    }

    fn description(&self) -> String {
        "Run a shell command in the background and return a job id immediately. \
         Use this for long work (builds, test suites, servers) that would otherwise be killed \
         by run_bash's 30s timeout. The command keeps running after this call returns: poll it \
         with jobs_wait or jobs_list, stop it with jobs_kill."
            .into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to run in the background."
                },
                "label": {
                    "type": "string",
                    "description": "Short name for the job, shown in jobs_list. Defaults to the command's first line."
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory. Defaults to the daemon's cwd."
                }
            },
            "required": ["command"]
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let command = need_str(args, "command")?;
        let label = opt_str(args, "label").unwrap_or_else(|| summarize(&command));
        let cwd = opt_str(args, "cwd");

        let task: jobs::JobTask =
            Box::new(move |control| run_command(&command, cwd.as_deref(), control));

        let id = self.reg.start(label.clone(), task).map_err(job_err)?;

        Ok(format!(
            "started job {} ({label})\n\
             It is running in the background. Use jobs_wait with this id to block until it \
             finishes, or jobs_list to see all jobs.",
            id.as_str()
        ))
    }
}

// -----------------------------------------------------------------------------
// jobs_wait
// -----------------------------------------------------------------------------

pub struct JobsWait {
    reg: Arc<LocalJobRegistry>,
}

impl JobsWait {
    pub fn new(reg: Arc<LocalJobRegistry>) -> Self {
        Self { reg }
    }
}

impl Tool for JobsWait {
    fn name(&self) -> &str {
        "jobs_wait"
    }

    fn description(&self) -> String {
        "Wait for a background job to finish and return its output. Blocks until the job completes \
         or the timeout (default 30s) elapses. On timeout the job keeps running — call jobs_wait \
         again to keep waiting."
            .into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Job id from jobs_start."},
                "timeout_ms": {
                    "type": "integer",
                    "description": "How long to wait before returning, in milliseconds. Default 30000."
                }
            },
            "required": ["id"]
        })
    }

    fn execute(&self, args: &Value, signal: &AtomicBool) -> Result<String, ToolError> {
        let raw = need_str(args, "id")?;
        let id = JobId::new(&raw);
        let timeout = opt_u64(args, "timeout_ms").unwrap_or(DEFAULT_WAIT_MS);

        // Wait in slices so an abort can cut the wait short. Aborting the wait
        // is explicitly *not* killing the job: the model asked to stop waiting,
        // not to stop the work, and conflating the two would let a cancelled
        // turn destroy background progress.
        let deadline = Instant::now() + Duration::from_millis(timeout);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                let summary = self.reg.status(&id).map_err(job_err)?;
                return Ok(format!(
                    "job {raw} is still {} after {timeout}ms. \
                     Call jobs_wait again to keep waiting, or jobs_kill to stop it.",
                    state_word(summary.state)
                ));
            }
            if signal.load(Ordering::Relaxed) {
                return Ok(format!(
                    "stopped waiting for job {raw}; it is still running. \
                     Call jobs_wait again, or jobs_kill to stop it."
                ));
            }
            let slice = remaining.min(WAIT_SLICE);
            match self.reg.wait(&id, Some(slice.as_millis() as u64)) {
                Ok(out) => return Ok(format_job_output(&raw, &out)),
                // Not finished within this slice: loop, which re-checks both the
                // deadline and the abort flag.
                Err(JobError::StillRunning(_)) => continue,
                Err(e) => return Err(job_err(e)),
            }
        }
    }
}

// -----------------------------------------------------------------------------
// jobs_list
// -----------------------------------------------------------------------------

pub struct JobsList {
    reg: Arc<LocalJobRegistry>,
}

impl JobsList {
    pub fn new(reg: Arc<LocalJobRegistry>) -> Self {
        Self { reg }
    }
}

impl Tool for JobsList {
    fn name(&self) -> &str {
        "jobs_list"
    }

    fn description(&self) -> String {
        "List background jobs, newest first, with their state and exit code.".into()
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    fn execute(&self, _args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let rows = self.reg.list();
        if rows.is_empty() {
            return Ok("no jobs".into());
        }
        let mut out = String::new();
        for row in rows {
            let code = match row.exit_code {
                Some(c) => format!(" exit={c}"),
                None => String::new(),
            };
            out.push_str(&format!(
                "{:<12} {:<9} {}{}\n",
                row.id.as_str(),
                state_word(row.state),
                row.label,
                code
            ));
        }
        Ok(out.trim_end().to_string())
    }
}

// -----------------------------------------------------------------------------
// jobs_kill
// -----------------------------------------------------------------------------

pub struct JobsKill {
    reg: Arc<LocalJobRegistry>,
}

impl JobsKill {
    pub fn new(reg: Arc<LocalJobRegistry>) -> Self {
        Self { reg }
    }
}

impl Tool for JobsKill {
    fn name(&self) -> &str {
        "jobs_kill"
    }

    fn description(&self) -> String {
        "Stop a running background job. Its recorded output stays available via jobs_wait. \
         Killing an already-finished job is harmless and reports how it ended."
            .into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"id": {"type": "string", "description": "Job id from jobs_start."}},
            "required": ["id"]
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let raw = need_str(args, "id")?;
        let id = JobId::new(&raw);
        // `kill` returns the state *before* the kill, which is what makes the
        // report honest: "was running, now killed" vs "had already finished".
        let before = self.reg.kill(&id).map_err(job_err)?;
        Ok(if before == JobState::Running {
            format!("killed job {raw}")
        } else {
            format!(
                "job {raw} had already {}; nothing to stop",
                state_word(before)
            )
        })
    }
}
