//! Out-of-process subagent backend speaking the Agent Client Protocol.
//!
//! Mirrors `deepseek-harness/packages/subagent/subagent-acp` (dsh): a child
//! agent binary is spawned, the ACP handshake runs over stdio NDJSON, and the
//! turn's outcome is published through [`SubagentProvider::start`]. The
//! child's lifetime is owned by [`AcpDisposer`] — see
//! [`crate::provider::RunDisposer`].
//!
//! # The dispose ladder
//!
//! `dispose` tears the child down in two tiers. dsh's ladder has three (EOF
//! then SIGTERM then grace then SIGKILL); the omenic cut drops the SIGTERM
//! step because std has no portable signal API — [`Child::kill`] is SIGKILL.
//! ponytail: restore the graceful middle tier by sending SIGTERM through
//! `libc` or `portable-pty` between tier 1 and tier 2 below; the ladder
//! already has the slot for it.
//!
//! 1. Cancel the outstanding request, close the pending-request table so any
//!    in-flight [`AcpClient::prompt`] unblocks, and drop the child's stdin so
//!    a cooperative agent sees EOF and exits.
//! 2. Poll the child for `eof_grace`; if it is still alive, SIGKILL it, poll
//!    for `kill_grace`, then `wait` to reap — std does not reap on drop on
//!    Unix, so an undisposed child would orphan.

use std::io::{self, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::acp::{
    AcpClient, AcpError, AcpHandlers, PermissionOutcome, PromptResponse, RequestPermissionRequest,
    RequestPermissionResponse,
};
use crate::provider::{
    RunDisposer, SubagentCapabilities, SubagentProvider, SubagentResult, SubagentRun,
    SubagentStartRequest,
};

/// `session/prompt` stop reasons that settle the turn without a failure.
const STOP_END_TURN: &str = "end_turn";
const STOP_CANCELLED: &str = "cancelled";

/// How often the exit watcher polls the child process.
const WATCHER_POLL: Duration = Duration::from_millis(50);

/// How an [`AcpProvider`] answers the child's permission requests.
///
/// ponytail: ACP options also carry a `kind` (allow_once / allow_always / …),
/// which dsh filters on; the omenic backend does not model it yet — `Allow`
/// takes the first option the child offers and falls back to `Deny` when the
/// child offers nothing. Add the kind filter when a real agent distinguishes
/// them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcpPermission {
    /// Approve the first offered option.
    Allow,
    /// Deny every permission request.
    Reject,
}

/// Harness-side description of an ACP agent binary.
///
/// Deliberately free of `config` types: the daemon layer (B4b) translates a
/// `SubagentProviderConfig` into this. Fields are exactly what the backend
/// needs to spawn and drive one turn.
#[derive(Clone, Debug)]
pub struct AcpProviderSpec {
    /// Command line, whitespace-split (no shell, no quoting).
    pub command: String,
    /// Working directory of the child process.
    pub cwd: Option<std::path::PathBuf>,
    /// Environment overrides applied on top of the parent's environment.
    pub env: Vec<(String, String)>,
    /// Policy for the child's permission requests.
    pub permission: AcpPermission,
    /// How long to wait for a cooperative exit after stdin EOF.
    pub eof_grace: Duration,
    /// How long to wait for the child to die after SIGKILL.
    pub kill_grace: Duration,
}

impl AcpProviderSpec {
    /// Sensible production defaults: generous graces, permissive policy.
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            cwd: None,
            env: Vec::new(),
            permission: AcpPermission::Allow,
            eof_grace: Duration::from_secs(6),
            kill_grace: Duration::from_secs(3),
        }
    }
}

/// Out-of-process ACP subagent backend.
///
/// A cheap spec handle ([`Clone`] over an [`Arc`]); all per-run state lives
/// on the worker thread spawned by [`SubagentProvider::start`]. One process,
/// one turn — same one-shot contract as the fork backend.
#[derive(Clone)]
pub struct AcpProvider {
    spec: Arc<AcpProviderSpec>,
}

impl AcpProvider {
    pub fn new(spec: AcpProviderSpec) -> Self {
        Self {
            spec: Arc::new(spec),
        }
    }
}

impl SubagentProvider for AcpProvider {
    fn name(&self) -> &str {
        "acp"
    }

    fn capabilities(&self) -> SubagentCapabilities {
        // The starting cut streams assistant text only: no output schema, no
        // depth budget, no tool filtering, no persona injection.
        SubagentCapabilities::default()
    }

    fn inherits_parent_context(&self) -> bool {
        false
    }

    fn start(&self, request: SubagentStartRequest) -> SubagentRun {
        let (tx, rx) = mpsc::channel();
        let spec = self.spec.clone();

        let mut command = match build_command(&spec) {
            Ok(command) => command,
            Err(error) => return failed_start(rx, tx, error),
        };
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => return failed_start(rx, tx, format!("spawn failed: {error}")),
        };

        // The child's stdin is shared with the disposer: taking it out gives
        // the child EOF (a `ChildStdin` drop closes the pipe). The client is
        // shared for the same reason — `close()` is the only way to unblock
        // an in-flight request, and `AcpClient` is not `Clone`.
        let stdout = child.stdout.take().expect("stdout was piped");
        let stdin = SharedStdin(Arc::new(Mutex::new(child.stdin.take())));
        let child = Arc::new(Mutex::new(Some(child)));
        let session_id = Arc::new(Mutex::new(None::<String>));
        let disposed = Arc::new(AtomicBool::new(false));

        let handlers = Arc::new(AcpHandlersImpl {
            permission: spec.permission,
            output: Mutex::new(String::new()),
        });
        let client = Arc::new(AcpClient::new(stdin.clone(), stdout, handlers.clone()));

        let disposer = Arc::new(AcpDisposer {
            signal: request.signal.clone(),
            client: client.clone(),
            stdin: stdin.clone(),
            child: child.clone(),
            session_id: session_id.clone(),
            disposed: disposed.clone(),
            eof_grace: spec.eof_grace,
            kill_grace: spec.kill_grace,
        });

        // `AcpClient` only unblocks an in-flight request when the pending
        // table is cleared, so a child that dies mid-turn would wedge
        // `prompt` forever. dsh gets this from its transport's end event; std
        // has no such hook, so we poll the process and close on exit.
        spawn_exit_watcher(child, client.clone());

        let worker_disposer = disposer.clone();
        let join = thread::spawn(move || {
            let result = run_turn(
                &client,
                &handlers,
                &spec,
                &request,
                &session_id,
                &worker_disposer,
            );
            let _ = tx.send(result);
        });

        SubagentRun::new(rx, join, disposer)
    }
}

/// Publish a startup failure and hand back a run with nothing to dispose of.
fn failed_start(
    rx: mpsc::Receiver<SubagentResult>,
    tx: mpsc::Sender<SubagentResult>,
    error: String,
) -> SubagentRun {
    let _ = tx.send(SubagentResult::Failed { error });
    SubagentRun::new(rx, thread::spawn(|| {}), Arc::new(NoopDisposer))
}

/// Disposer for a run whose child never existed.
struct NoopDisposer;

impl RunDisposer for NoopDisposer {
    fn dispose(&self) {}
}

/// Build the child process from the spec (no shell involved).
fn build_command(spec: &AcpProviderSpec) -> Result<Command, String> {
    let mut parts = spec.command.split_whitespace();
    let program = parts
        .next()
        .ok_or_else(|| "empty agent command".to_string())?;
    let mut command = Command::new(program);
    command.args(parts);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    if let Some(cwd) = &spec.cwd {
        command.current_dir(cwd);
    }
    for (key, value) in &spec.env {
        command.env(key, value);
    }
    Ok(command)
}

/// stdin endpoint the disposer can take away.
///
/// `Write` after the take reports `BrokenPipe` — deliberately not `Ok(0)`,
/// which would make `write_all` spin on a closed pipe.
#[derive(Clone)]
struct SharedStdin(Arc<Mutex<Option<ChildStdin>>>);

impl SharedStdin {
    /// Drop the pipe; the child reads EOF.
    fn take(&self) -> Option<ChildStdin> {
        self.0.lock().unwrap().take()
    }
}

impl Write for SharedStdin {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap()
            .as_mut()
            .map(|stdin| stdin.write(buf))
            .unwrap_or_else(|| Err(io::Error::from(io::ErrorKind::BrokenPipe)))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0
            .lock()
            .unwrap()
            .as_mut()
            .map(|stdin| stdin.flush())
            .unwrap_or(Ok(()))
    }
}

/// Runs the ACP transaction and maps the outcome to a [`SubagentResult`].
fn run_turn(
    client: &AcpClient<SharedStdin, ChildStdout>,
    handlers: &AcpHandlersImpl,
    spec: &AcpProviderSpec,
    request: &SubagentStartRequest,
    session_id: &Mutex<Option<String>>,
    disposer: &AcpDisposer,
) -> SubagentResult {
    // A caller who aborted before we spoke to the child never wanted this
    // turn; do not hand the child a prompt to act on.
    if request.signal.load(Ordering::Relaxed) {
        disposer.dispose();
        return SubagentResult::Aborted;
    }

    if let Err(error) = client.initialize() {
        return rollback("initialize failed", error, disposer);
    }
    let session = match client.new_session(&cwd_string(spec)) {
        Ok(session) => session,
        Err(error) => return rollback("session/new failed", error, disposer),
    };
    if session.session_id.is_empty() {
        return rollback(
            "session/new returned no session id",
            AcpError::Protocol("empty session id".into(), None),
            disposer,
        );
    }
    *session_id.lock().unwrap() = Some(session.session_id.clone());

    match client.prompt(&session.session_id, &request.prompt) {
        Ok(response) => map_prompt_response(response, &handlers.output.lock().unwrap()),
        Err(AcpError::ChannelClosed) => {
            // The channel was closed under us: either dispose (our own
            // teardown) or the exit watcher (the child is gone). Only the
            // first is an abort.
            if disposer.disposed.load(Ordering::Relaxed) {
                SubagentResult::Aborted
            } else {
                SubagentResult::Failed {
                    error: "agent process exited without finishing the turn".into(),
                }
            }
        }
        Err(error) => SubagentResult::Failed {
            error: format!("prompt failed: {error}"),
        },
    }
}

/// Map the agent's terminal stop reason onto a [`SubagentResult`] (dsh's
/// `acpStopReason`): `end_turn` completes, `cancelled` aborts, anything else
/// — including a missing reason — is a failure, never a silent success.
fn map_prompt_response(response: PromptResponse, output: &str) -> SubagentResult {
    match response.stop_reason.as_ref() {
        Some(reason) if reason.as_str() == STOP_END_TURN => SubagentResult::Completed {
            output: output.to_string(),
        },
        Some(reason) if reason.as_str() == STOP_CANCELLED => SubagentResult::Aborted,
        Some(reason) => SubagentResult::Failed {
            error: format!("agent ended the turn with reason '{}'", reason.as_str()),
        },
        None => SubagentResult::Failed {
            error: "agent ended the turn without a stop reason".into(),
        },
    }
}

/// Tear the still-live child down and report the startup failure (dsh's
/// startup rollback owns the process).
fn rollback(context: &str, error: AcpError, disposer: &AcpDisposer) -> SubagentResult {
    disposer.dispose();
    SubagentResult::Failed {
        error: format!("{context}: {error}"),
    }
}

fn cwd_string(spec: &AcpProviderSpec) -> String {
    spec.cwd
        .as_ref()
        .and_then(|cwd| cwd.to_str())
        .unwrap_or("")
        .to_string()
}

fn spawn_exit_watcher(
    child: Arc<Mutex<Option<Child>>>,
    client: Arc<AcpClient<SharedStdin, ChildStdout>>,
) {
    thread::spawn(move || {
        loop {
            let exited = match child.lock().unwrap().as_mut() {
                Some(child) => child.try_wait().map_or(true, |status| status.is_some()),
                None => true,
            };
            if exited {
                let _ = client.close();
                break;
            }
            thread::sleep(WATCHER_POLL);
        }
    });
}

/// Accumulates the agent's streamed text and answers its permission requests.
struct AcpHandlersImpl {
    permission: AcpPermission,
    output: Mutex<String>,
}

impl AcpHandlers for AcpHandlersImpl {
    fn on_message_chunk(&self, _session_id: &str, text: &str) {
        self.output.lock().unwrap().push_str(text);
    }

    fn on_permission(&self, request: RequestPermissionRequest) -> RequestPermissionResponse {
        match self.permission {
            AcpPermission::Reject => RequestPermissionResponse {
                outcome: PermissionOutcome::Deny,
            },
            AcpPermission::Allow => match request.options.first() {
                Some(option) => RequestPermissionResponse {
                    outcome: PermissionOutcome::Allow {
                        option_id: option.option_id.clone(),
                        modified_call: None,
                    },
                },
                // The child offered nothing an allow policy can pick.
                None => RequestPermissionResponse {
                    outcome: PermissionOutcome::Deny,
                },
            },
        }
    }
}

/// Owns the child process for a single run: the two-tier ladder in the module
/// docs. Idempotent — the worker calls it to roll back a failed startup, the
/// caller calls it to interrupt, and only one of them runs the ladder.
struct AcpDisposer {
    /// The request's abort flag: flipping it makes the worker's startup gate
    /// (which checks the same flag) short-circuit instead of writing to a
    /// stdin this disposer has already taken. Same semantics as `ForkDisposer`.
    signal: std::sync::Arc<std::sync::atomic::AtomicBool>,
    client: Arc<AcpClient<SharedStdin, ChildStdout>>,
    stdin: SharedStdin,
    child: Arc<Mutex<Option<Child>>>,
    session_id: Arc<Mutex<Option<String>>>,
    /// Also the worker's abort-vs-failure marker for a closed channel.
    disposed: Arc<AtomicBool>,
    eof_grace: Duration,
    kill_grace: Duration,
}

impl RunDisposer for AcpDisposer {
    fn dispose(&self) {
        if self.disposed.swap(true, Ordering::Relaxed) {
            return;
        }

        // The worker's startup gate reads this flag; set it first so a
        // dispose that won the startup race cannot race the worker's first
        // write.
        self.signal.store(true, Ordering::Relaxed);

        // Cooperative: a live agent may settle the turn itself on cancel.
        if let Some(session_id) = self.session_id.lock().unwrap().clone() {
            let _ = self.client.cancel(&session_id);
        }
        // Then unblock whatever the worker is blocked on and give the child
        // EOF — a cooperative agent flushes and exits here.
        let _ = self.client.close();
        let _ = self.stdin.take();

        if !self.wait_for_exit(self.eof_grace) {
            // ponytail: dsh sends SIGTERM here and waits its term grace
            // before SIGKILL. std has no portable signal API, so the
            // graceful tier is skipped; add `libc`/`portable-pty` to restore.
            let _ = self
                .child
                .lock()
                .unwrap()
                .as_mut()
                .map(|child| child.kill());
            let _ = self.wait_for_exit(self.kill_grace);
        }

        // std does not reap on drop on Unix. SIGKILL has been delivered (or
        // the child already exited), so this is prompt; `try_wait` above may
        // have reaped it already, in which case there is nothing left to own.
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.wait();
        }
    }
}

impl AcpDisposer {
    /// Poll the child until it exits or the timeout elapses. `try_wait`
    /// reaps on Unix, so a successful poll leaves no zombie.
    fn wait_for_exit(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            match self.child.lock().unwrap().as_mut() {
                Some(child) => match child.try_wait() {
                    Ok(Some(_)) => return true,
                    Ok(None) => {}
                    // Already reaped by the exit watcher or a racing dispose.
                    Err(_) => return true,
                },
                // The child is already taken care of.
                None => return true,
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(WATCHER_POLL);
        }
    }
}
