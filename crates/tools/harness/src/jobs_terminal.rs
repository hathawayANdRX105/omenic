//! Model-facing tools for background jobs and persistent terminals.
//!
//! Two capabilities that a single `run_bash` call cannot express:
//!
//! * **Background jobs** — [`omenic_harness_jobs`]. `run_bash` is bounded by a
//!   30s timeout (`crates/agent/tools/src/bash.rs`), so a build or a test suite
//!   that runs longer is killed mid-flight with no way to observe it. A job is
//!   started via `jobs_start`, returns an id immediately, and is polled with
//!   `jobs_wait` / `jobs_list`.
//! * **Persistent terminals** — [`omenic_harness_terminal`]. Each `run_bash`
//!   call gets a fresh shell, so `cd`, exported variables and an interactive
//!   program cannot survive between calls. A terminal session holds a real pty
//!   open across calls.
//!
//! # Which `Tool` trait
//!
//! These implement the **agent-domain** `tools::Tool` (name / description /
//! parameters / execute), not the harness `protocol::Tool`. That is
//! deliberate: the engine dispatches `Box<dyn tools::Tool>`, and MCP tools
//! arrive in that same shape, so implementing it here means the daemon hands
//! these to the engine directly with no adapter. (The two traits stay separate
//! by design — C6; bridging them is the daemon's job, and this crate only has to
//! agree with the side the engine consumes.)
//!
//! # Sharing
//!
//! Both registries are daemon-lifetime singletons, not per-call state: a job
//! started in one turn must still be visible in the next, and a terminal session
//! must outlive the tool call that created it. The tools therefore hold
//! `Arc<...>` handles and are built once by the daemon, then handed to every
//! engine (re)spawn as `Arc<dyn tools::Tool>` — the same shape the daemon
//! already uses for MCP tools. Cloning a tool clones the `Arc`, so every engine
//! sees one registry.
//!
//! # The abort signal
//!
//! `jobs_start`, `terminal_create` and `terminal_write` deliberately **ignore**
//! the abort signal: their whole purpose is work that outlives the call.
//! Stopping the work is what `jobs_kill` / `terminal_kill` are for. `jobs_wait`
//! alone reads the signal, and reads it as a *wait* cutoff — aborting the wait
//! returns to the model without killing the job. "Stop waiting" and "stop the
//! work" are different requests and get different tools.

use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use jobs::{JobControl, JobError, JobOutput, JobState, LocalJobRegistry};
use serde_json::{Value, json};
use terminal::{TerminalError, TerminalRegistry};
use tools::{Tool, ToolError};

mod job;
mod tty;

pub use job::*;
pub use tty::*;

/// Poll interval shared by job supervision and `jobs_wait`'s abort check.
///
/// The job registry's own `wait` blocks on a condvar internally; this slice
/// exists only so the abort flag is re-read often enough to stay responsive.
const WAIT_SLICE: Duration = Duration::from_millis(50);

/// Default `jobs_wait` timeout when the caller does not pass one.
const DEFAULT_WAIT_MS: u64 = 30_000;

/// Cap on the bytes one `terminal_read` may return, so a chatty process cannot
/// flood the model's context in a single call. The registry keeps a larger
/// buffer; this is a per-call view onto it.
const READ_CHUNK_BYTES: usize = 16 * 1024;

// -----------------------------------------------------------------------------
// Argument helpers
// -----------------------------------------------------------------------------

fn need_str(args: &Value, key: &str) -> Result<String, ToolError> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ToolError::Message(format!("missing string argument: {key}")))
}

fn opt_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_string)
}

fn opt_u64(args: &Value, key: &str) -> Option<u64> {
    args.get(key).and_then(Value::as_u64)
}

fn need_u16(args: &Value, key: &str) -> Result<u16, ToolError> {
    let n = args
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| ToolError::Message(format!("missing integer argument: {key}")))?;
    u16::try_from(n).map_err(|_| ToolError::Message(format!("{key} out of range: {n}")))
}

/// Read an optional pty dimension, defaulting to `def`.
///
/// `as u16` is the tempting one-liner and the wrong one: it silently wraps a
/// `cols` of 70000 into a geometry nobody asked for. [`need_u16`] validates
/// the required case; this covers the optional one the same way.
fn opt_dim(args: &Value, key: &str, def: u16) -> Result<u16, ToolError> {
    opt_u64(args, key)
        .map(|n| {
            u16::try_from(n).map_err(|_| ToolError::Message(format!("{key} out of range: {n}")))
        })
        .transpose()
        .map(|dim| dim.unwrap_or(def))
}

// -----------------------------------------------------------------------------
// Error mapping
// -----------------------------------------------------------------------------

fn job_err(e: JobError) -> ToolError {
    match e {
        JobError::Unknown(id) => ToolError::Message(format!("unknown job: {}", id.as_str())),
        JobError::StillRunning(id) => {
            ToolError::Message(format!("job {} is still running", id.as_str()))
        }
        JobError::Refused(msg) => ToolError::Message(msg),
    }
}

fn terminal_err(e: TerminalError) -> ToolError {
    match e {
        TerminalError::Unknown(id) => {
            ToolError::Message(format!("unknown terminal: {}", id.as_str()))
        }
        TerminalError::Pty(msg) => ToolError::Message(format!("pty error: {msg}")),
        TerminalError::Exited(id) => {
            ToolError::Message(format!("terminal {} has exited", id.as_str()))
        }
        TerminalError::Refused(msg) => ToolError::Message(msg),
    }
}

// -----------------------------------------------------------------------------
// Small formatting helpers
// -----------------------------------------------------------------------------

/// A one-line label derived from a command, for jobs the caller did not name.
fn summarize(command: &str) -> String {
    let first_line = command.lines().next().unwrap_or("").trim();
    if first_line.chars().count() <= 60 {
        first_line.to_string()
    } else {
        format!("{}…", first_line.chars().take(59).collect::<String>())
    }
}

fn state_word(s: JobState) -> &'static str {
    match s {
        JobState::Running => "running",
        JobState::Completed => "completed",
        JobState::Failed => "failed",
        JobState::Killed => "killed",
    }
}

fn format_job_output(id: &str, out: &JobOutput) -> String {
    let mut s = format!("job {id} finished");
    if let Some(code) = out.exit_code {
        s.push_str(&format!(" with exit code {code}"));
    }
    s.push('\n');
    let stdout = out.stdout.trim_end();
    let stderr = out.stderr.trim_end();
    if !stdout.is_empty() {
        s.push_str("--- stdout ---\n");
        s.push_str(stdout);
        s.push('\n');
    }
    if !stderr.is_empty() {
        s.push_str("--- stderr ---\n");
        s.push_str(stderr);
        s.push('\n');
    }
    if stdout.is_empty() && stderr.is_empty() {
        s.push_str("(no output)\n");
    }
    s
}

/// Cut `s` to at most `max` bytes on a char boundary.
///
/// Returns the (possibly shortened) text and whether it is **complete** — i.e.
/// nothing was dropped. The flag is named for the caller's question ("did I get
/// all of it?") rather than for the operation, because an inverted boolean here
/// would silently invert the truncation notice.
fn truncate_bytes(s: &str, max: usize) -> (String, bool) {
    if s.len() <= max {
        return (s.to_string(), true);
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (s[..end].to_string(), false)
}

/// The daemon's cwd, used when the caller does not pass one.
fn current_dir() -> String {
    std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ".".into())
}

// -----------------------------------------------------------------------------
// Running a job's command
// -----------------------------------------------------------------------------

/// Run `command` under `sh -c`, honouring the job's cancellation flag.
///
/// Cancellation is polled rather than signalled: the job is an arbitrary shell
/// pipeline, and killing just the `sh -c` child would leave its own children
/// running. The child is placed in its own process group so a cancel can take
/// the whole tree down.
fn run_command(
    command: &str,
    cwd: Option<&str>,
    control: &JobControl,
) -> Result<JobOutput, String> {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Own process group, so `kill_tree` reaches grandchildren.
        cmd.process_group(0);
    }

    let mut child = cmd.spawn().map_err(|e| format!("failed to spawn: {e}"))?;
    let pid = child.id();

    // Drain both pipes on their own threads: a child that fills stderr while we
    // block on stdout would otherwise deadlock.
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let out_reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = Vec::new();
        if let Some(p) = stdout_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });
    let err_reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = Vec::new();
        if let Some(p) = stderr_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });

    let status = loop {
        if control.is_cancelled() {
            // Best-effort: the group may already be gone. `kill_tree` logs a
            // real failure; the run still reports `Killed` because the control
            // flag is what the registry records.
            let _ = kill_tree(pid);
            let _ = child.wait();
            break None;
        }
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => std::thread::sleep(WAIT_SLICE),
            Err(e) => return Err(format!("wait failed: {e}")),
        }
    };

    // A reader thread can only fail by panicking, and a silent empty string
    // is indistinguishable from "the command printed nothing" — so say which
    // pipe was lost instead of quietly returning half a result.
    let drain = |h: std::thread::JoinHandle<Vec<u8>>, which: &str| -> String {
        match h.join() {
            Ok(buf) => String::from_utf8_lossy(&buf).into_owned(),
            Err(_) => format!("[omenic] the {which} reader thread panicked; output lost"),
        }
    };
    let stdout = drain(out_reader, "stdout");
    let stderr = drain(err_reader, "stderr");

    // A cancelled run reports the output captured before the kill; the registry
    // records the `Killed` state from the control flag, not from here.
    Ok(JobOutput {
        stdout,
        stderr,
        exit_code: status.and_then(|s| s.code()),
    })
}

/// Kill a child and everything in its process group.
///
/// Returns whether the group signal was delivered. A failure is worth
/// surfacing rather than discarding: `ESRCH` just means the group already
/// exited, but `EPERM` means it is still alive and a cancelled job would be
/// reported as killed while its descendants keep running.
#[cfg(unix)]
fn kill_tree(pid: u32) -> bool {
    // Negative pid targets the group; SIGKILL because the registry's `kill` is
    // an explicit "stop now" and a shell ignoring SIGTERM would keep the group
    // alive.
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    let rc = unsafe { kill(-(pid as i32), 9) };
    if rc == 0 {
        return true;
    }
    let err = std::io::Error::last_os_error();
    // `ESRCH` is the ordinary case: the group finished between `try_wait` and
    // here. Anything else means the group outlived the kill.
    let gone = err.raw_os_error() == Some(3);
    if !gone {
        eprintln!("omenic: kill(-{pid}, SIGKILL) failed: {err}");
    }
    gone
}

#[cfg(not(unix))]
fn kill_tree(pid: u32) -> bool {
    // No process groups to reach for on this platform; the pid is unused.
    let _ = pid;
    false
}

// -----------------------------------------------------------------------------
// Registration
// -----------------------------------------------------------------------------

/// Every job/terminal tool, backed by the given shared registries.
///
/// The daemon calls this once and shares the resulting handles across engine
/// respawns (see the module docs). Returned as `Arc<dyn Tool>` so a clone is a
/// pointer copy and every engine observes the same registries.
pub fn session_tools(
    jobs: Arc<LocalJobRegistry>,
    terminals: Arc<TerminalRegistry>,
) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(JobsStart::new(Arc::clone(&jobs))),
        Arc::new(JobsWait::new(Arc::clone(&jobs))),
        Arc::new(JobsList::new(Arc::clone(&jobs))),
        Arc::new(JobsKill::new(Arc::clone(&jobs))),
        Arc::new(TerminalCreate::new(Arc::clone(&terminals))),
        Arc::new(TerminalWrite::new(Arc::clone(&terminals))),
        Arc::new(TerminalRead::new(Arc::clone(&terminals))),
        Arc::new(TerminalResize::new(Arc::clone(&terminals))),
        Arc::new(TerminalKill::new(Arc::clone(&terminals))),
        Arc::new(TerminalList::new(terminals)),
    ]
}

/// Names of every tool [`session_tools`] registers, in registration order.
///
/// Exposed so the daemon and its tests can assert on the set without building
/// registries, and so the web UI's tool-name vocabulary can be checked against
/// the real list rather than a hand-copied duplicate.
pub const SESSION_TOOL_NAMES: &[&str] = &[
    "jobs_start",
    "jobs_wait",
    "jobs_list",
    "jobs_kill",
    "terminal_create",
    "terminal_write",
    "terminal_read",
    "terminal_resize",
    "terminal_kill",
    "terminal_list",
];
