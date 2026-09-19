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
//! parameters / execute), not the harness `omenic_harness_core::Tool`. That is
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

use omenic_harness_jobs::{
    JobControl, JobError, JobId, JobOutput, JobRegistry, JobState, LocalJobRegistry,
};
use omenic_harness_terminal::{TerminalError, TerminalId, TerminalRegistry};
use serde_json::{Value, json};
use tools::{Tool, ToolError};

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
            kill_tree(pid);
            let _ = child.wait();
            break None;
        }
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => std::thread::sleep(WAIT_SLICE),
            Err(e) => return Err(format!("wait failed: {e}")),
        }
    };

    let stdout = String::from_utf8_lossy(&out_reader.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err_reader.join().unwrap_or_default()).into_owned();

    // A cancelled run reports the output captured before the kill; the registry
    // records the `Killed` state from the control flag, not from here.
    Ok(JobOutput {
        stdout,
        stderr,
        exit_code: status.and_then(|s| s.code()),
    })
}

/// Kill a child and everything in its process group.
#[cfg(unix)]
fn kill_tree(pid: u32) {
    // Negative pid targets the group; SIGKILL because the registry's `kill` is
    // an explicit "stop now" and a shell ignoring SIGTERM would keep the group
    // alive.
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    unsafe {
        kill(-(pid as i32), 9);
    }
}

#[cfg(not(unix))]
fn kill_tree(pid: u32) {
    // No process groups to reach for on this platform; the pid is unused.
    let _ = pid;
}

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

        let task: omenic_harness_jobs::JobTask =
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

// -----------------------------------------------------------------------------
// terminal_create
// -----------------------------------------------------------------------------

pub struct TerminalCreate {
    reg: Arc<TerminalRegistry>,
}

impl TerminalCreate {
    pub fn new(reg: Arc<TerminalRegistry>) -> Self {
        Self { reg }
    }
}

impl Tool for TerminalCreate {
    fn name(&self) -> &str {
        "terminal_create"
    }

    fn description(&self) -> String {
        "Open a persistent shell session and return its id. Unlike run_bash, state survives \
         between calls: cd, exported variables and running programs persist until the session is \
         closed. Use terminal_write to send commands and terminal_read to see output."
            .into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "shell": {
                    "type": "string",
                    "description": "Shell to launch, with any flags. Defaults to bash."
                },
                "cwd": {
                    "type": "string",
                    "description": "Initial working directory. Defaults to the daemon's cwd."
                },
                "cols": {"type": "integer", "description": "Terminal width. Default 80."},
                "rows": {"type": "integer", "description": "Terminal height. Default 24."}
            }
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let shell = opt_str(args, "shell").unwrap_or_else(|| "bash".to_string());
        let cwd = opt_str(args, "cwd").unwrap_or_else(current_dir);
        let cols = opt_u64(args, "cols").unwrap_or(80) as u16;
        let rows = opt_u64(args, "rows").unwrap_or(24) as u16;

        let id = self
            .reg
            .create(&shell, &cwd, cols, rows)
            .map_err(terminal_err)?;
        Ok(format!(
            "terminal {} opened ({shell} in {cwd}, {cols}x{rows})\n\
             Send commands with terminal_write, then read output with terminal_read. \
             The shell echoes input, so output includes the command line itself.",
            id.as_str()
        ))
    }
}

// -----------------------------------------------------------------------------
// terminal_write / terminal_read
// -----------------------------------------------------------------------------

pub struct TerminalWrite {
    reg: Arc<TerminalRegistry>,
}

impl TerminalWrite {
    pub fn new(reg: Arc<TerminalRegistry>) -> Self {
        Self { reg }
    }
}

impl Tool for TerminalWrite {
    fn name(&self) -> &str {
        "terminal_write"
    }

    fn description(&self) -> String {
        "Send input to a terminal session. Include a trailing newline to run a command. The shell \
         echoes what you send, so terminal_read will show your input as well as its output."
            .into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Terminal id from terminal_create."},
                "data": {"type": "string", "description": "Text to send. End with a newline to execute."}
            },
            "required": ["id", "data"]
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let raw = need_str(args, "id")?;
        let data = need_str(args, "data")?;
        let id = TerminalId::new(&raw);
        self.reg.write(&id, &data).map_err(terminal_err)?;
        Ok(format!("wrote {} bytes to terminal {raw}", data.len()))
    }
}

pub struct TerminalRead {
    reg: Arc<TerminalRegistry>,
}

impl TerminalRead {
    pub fn new(reg: Arc<TerminalRegistry>) -> Self {
        Self { reg }
    }
}

impl Tool for TerminalRead {
    fn name(&self) -> &str {
        "terminal_read"
    }

    fn description(&self) -> String {
        "Read output produced since the last read and clear it. Returns promptly even when the \
         shell is idle — it does not wait for output unless you pass timeout_ms. Call it in a loop \
         after terminal_write to collect a command's result."
            .into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Terminal id from terminal_create."},
                "timeout_ms": {
                    "type": "integer",
                    "description": "How long to wait for new output before returning, in milliseconds. Default 0 (return immediately)."
                }
            },
            "required": ["id"]
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let raw = need_str(args, "id")?;
        let timeout = opt_u64(args, "timeout_ms").unwrap_or(0);
        let id = TerminalId::new(&raw);
        let out = self.reg.read(&id, Some(timeout)).map_err(terminal_err)?;

        let text = out.text();
        let (text, complete) = truncate_bytes(&text, READ_CHUNK_BYTES);
        let mut body = if text.is_empty() {
            "(no new output)".to_string()
        } else {
            text
        };
        if !complete {
            body.push_str("\n… (output truncated; call terminal_read again for more)");
        }
        if out.exited {
            body.push_str("\n(terminal exited)");
        }
        Ok(body)
    }
}

// -----------------------------------------------------------------------------
// terminal_resize / terminal_kill / terminal_list
// -----------------------------------------------------------------------------

pub struct TerminalResize {
    reg: Arc<TerminalRegistry>,
}

impl TerminalResize {
    pub fn new(reg: Arc<TerminalRegistry>) -> Self {
        Self { reg }
    }
}

impl Tool for TerminalResize {
    fn name(&self) -> &str {
        "terminal_resize"
    }

    fn description(&self) -> String {
        "Change a terminal's reported width and height. Full-screen programs (editors, pagers) \
         lay themselves out to these dimensions."
            .into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string"},
                "cols": {"type": "integer"},
                "rows": {"type": "integer"}
            },
            "required": ["id", "cols", "rows"]
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let raw = need_str(args, "id")?;
        let cols = need_u16(args, "cols")?;
        let rows = need_u16(args, "rows")?;
        let id = TerminalId::new(&raw);
        self.reg.resize(&id, cols, rows).map_err(terminal_err)?;
        Ok(format!("terminal {raw} resized to {cols}x{rows}"))
    }
}

pub struct TerminalKill {
    reg: Arc<TerminalRegistry>,
}

impl TerminalKill {
    pub fn new(reg: Arc<TerminalRegistry>) -> Self {
        Self { reg }
    }
}

impl Tool for TerminalKill {
    fn name(&self) -> &str {
        "terminal_kill"
    }

    fn description(&self) -> String {
        "Close a terminal session and kill its shell. Any process still running in it is \
         terminated."
            .into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"id": {"type": "string"}},
            "required": ["id"]
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let raw = need_str(args, "id")?;
        let id = TerminalId::new(&raw);
        self.reg.close(&id).map_err(terminal_err)?;
        Ok(format!("closed terminal {raw}"))
    }
}

pub struct TerminalList {
    reg: Arc<TerminalRegistry>,
}

impl TerminalList {
    pub fn new(reg: Arc<TerminalRegistry>) -> Self {
        Self { reg }
    }
}

impl Tool for TerminalList {
    fn name(&self) -> &str {
        "terminal_list"
    }

    fn description(&self) -> String {
        "List open terminal sessions with their shell, working directory and size.".into()
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    fn execute(&self, _args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let rows = self.reg.list();
        if rows.is_empty() {
            return Ok("no terminals".into());
        }
        let mut out = String::new();
        for row in rows {
            let ended = if row.exited { " (exited)" } else { "" };
            out.push_str(&format!(
                "{:<12} {:<24} {:<28} {}x{}{}\n",
                row.id.as_str(),
                row.shell,
                row.cwd,
                row.cols,
                row.rows,
                ended
            ));
        }
        Ok(out.trim_end().to_string())
    }
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
