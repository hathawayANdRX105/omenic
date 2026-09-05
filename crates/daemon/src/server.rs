//! Daemon orchestrator: acquire lock, bind socket, accept loop, drop = clean
//! shutdown.
//!
//! Public surface:
//!
//! * [`DaemonConfig`] — what you need to start one.
//! * [`Daemon::start`] — bring it up.
//! * `Daemon` owns the lock + worker state; `Drop` performs the
//!   shutdown sequence so accidental early-return cleanup is automatic.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::DaemonError;
use crate::dispatch::{DispatchCtx, WorkerHandle};
use crate::lock::InstanceLock;
use crate::protocol::{Request, Response, ResponseError};
use crate::socket::{Connection, Listener, SocketAddr};
use crate::state::{RunLedger, SessionState, now_ms};

/// Knobs for `Daemon::start`.  All paths default to "ask `Config`"; supply
/// overrides for tests.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    /// Unix-domain socket path.  When `None`, falls back to
    /// `Config::daemon_socket_path()` (via env).
    pub socket_path: Option<PathBuf>,
    /// Path to the omp binary handed to `rpc::worker::Worker::new`.
    pub omp_path: String,
    /// Session database file path.  When `None`, the daemon refuses to
    /// start — there is no default and we don't want to silently create one
    /// in the current directory.
    pub session_db_path: Option<PathBuf>,
}

impl DaemonConfig {
    /// Resolve paths from `Config` and the runtime environment.
    pub fn from_config(cfg: &config::Config) -> Result<Self, DaemonError> {
        let socket_path = Some(cfg.daemon_socket_path()?);
        let session_db_path = Some(cfg.session_db_path()?);
        let omp_path = cfg.omp_path.to_string_lossy().into_owned();
        Ok(DaemonConfig {
            socket_path,
            omp_path,
            session_db_path,
        })
    }
}

/// Long-lived daemon.  Owned by the caller; dropping it cleans up.
pub struct Daemon {
    pub(crate) socket: SocketAddr,
    pub(crate) _lock: InstanceLock,
    pub(crate) session_state: SessionState,
    pub(crate) run_ledger: RunLedger,
    pub(crate) worker: Arc<Mutex<WorkerHandle>>,
    pub(crate) shutdown: Arc<AtomicBool>,
    pub(crate) started_at_ms: i64,
    pub(crate) accept_thread: Option<thread::JoinHandle<()>>,
}

impl Daemon {
    /// Time at which the daemon was started, in unix epoch milliseconds.
    pub fn started_at_ms(&self) -> i64 {
        self.started_at_ms
    }
    /// Start a daemon: lock, bind socket, open SessionDb, launch the accept
    /// loop on a background thread.
    pub fn start(cfg: DaemonConfig) -> Result<Self, DaemonError> {
        let socket_path = cfg
            .socket_path
            .as_ref()
            .ok_or_else(|| DaemonError::Protocol("socket_path is required".into()))?
            .clone();
        let session_db_path = cfg
            .session_db_path
            .as_ref()
            .ok_or_else(|| DaemonError::Protocol("session_db_path is required".into()))?
            .clone();

        let lock = InstanceLock::acquire(&socket_path)?;
        let listener = Listener::bind(&socket_path)?;
        let session_db = session::SessionDb::open(&session_db_path)?;
        let run_ledger = RunLedger::open_for_socket(&socket_path)?;

        let session_state = SessionState::new(session_db);
        let worker = Arc::new(Mutex::new(WorkerHandle::new(cfg.omp_path.clone())));
        let shutdown = Arc::new(AtomicBool::new(false));
        let started_at_ms = now_ms();

        let accept_thread = spawn_accept_loop(AcceptLoopCtx {
            listener,
            worker: Arc::clone(&worker),
            sessions: session_state.clone(),
            runs: run_ledger.clone(),
            shutdown: Arc::clone(&shutdown),
            started_at_ms,
        })?;

        Ok(Daemon {
            socket: SocketAddr::new(socket_path),
            _lock: lock,
            session_state,
            run_ledger,
            worker,
            shutdown,
            started_at_ms,
            accept_thread: Some(accept_thread),
        })
    }

    /// Trigger a graceful shutdown.  Sets the shutdown flag and waits for
    /// the accept thread to finish.  Idempotent.
    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }
    }

    /// Whether shutdown has been requested by a signal or daemon command.
    pub fn is_shutdown_requested(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    /// Socket address the daemon is listening on.
    pub fn socket_addr(&self) -> &SocketAddr {
        &self.socket
    }

    /// PID of the daemon process (from the pid file).
    pub fn pid(&self) -> u32 {
        self._lock.pid()
    }

    /// Handle to the underlying session DB.  Useful in tests for asserting
    /// on persisted state.
    pub fn sessions(&self) -> &SessionState {
        &self.session_state
    }

    /// Handle to the run ledger.  Useful in tests for asserting on
    /// recorded runs.
    pub fn runs(&self) -> &RunLedger {
        &self.run_ledger
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        // Order matters: tell the loop to stop, then join the thread,
        // then drop the lock (which removes pid + lock files).
        // The listener is owned by the accept loop thread and will be
        // dropped when the thread exits, cleaning up the socket file.
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }
        // A worker prompt may be blocked forever in an external process.
        // Never turn daemon shutdown into an unbounded mutex wait.
        if let Ok(mut worker) = self.worker.try_lock() {
            worker.reset();
        }
    }
}

struct AcceptLoopCtx {
    listener: Listener,
    worker: Arc<Mutex<WorkerHandle>>,
    sessions: SessionState,
    runs: RunLedger,
    shutdown: Arc<AtomicBool>,
    started_at_ms: i64,
}

fn spawn_accept_loop(ctx: AcceptLoopCtx) -> Result<thread::JoinHandle<()>, DaemonError> {
    let listener = ctx.listener;
    let worker = ctx.worker;
    let sessions = ctx.sessions;
    let runs = ctx.runs;
    let shutdown = ctx.shutdown;
    let started_at_ms = ctx.started_at_ms;

    let handle = thread::Builder::new()
        .name("omenic-daemon-accept".into())
        .spawn(move || {
            run_accept_loop(listener, worker, sessions, runs, shutdown, started_at_ms);
        })
        .map_err(DaemonError::Io)?;
    Ok(handle)
}

fn run_accept_loop(
    listener: Listener,
    worker: Arc<Mutex<WorkerHandle>>,
    sessions: SessionState,
    runs: RunLedger,
    shutdown: Arc<AtomicBool>,
    started_at_ms: i64,
) {
    // We poll the shutdown flag between accepts and use a short accept
    // timeout so we don't block forever once shutdown is signalled.
    let poll_interval = Duration::from_millis(50);
    while !shutdown.load(Ordering::SeqCst) {
        let conn = match listener.accept_timeout(poll_interval) {
            Ok(Some(c)) => c,
            Ok(None) => continue, // timeout — re-check shutdown flag
            Err(_) => break,      // listener closed or poisoned
        };
        let worker = Arc::clone(&worker);
        let sessions = sessions.clone();
        let runs = runs.clone();
        thread::spawn(move || {
            if let Err(e) = handle_connection(conn, &worker, &sessions, &runs, started_at_ms) {
                eprintln!("daemon: connection error: {e}");
            }
        });
    }
}

fn handle_connection(
    mut conn: Connection,
    worker: &Arc<Mutex<WorkerHandle>>,
    sessions: &SessionState,
    runs: &RunLedger,
    started_at_ms: i64,
) -> Result<(), DaemonError> {
    loop {
        let line = match conn.read_frame()? {
            Some(l) => l,
            None => return Ok(()), // EOF
        };
        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = Response::err(
                    None,
                    ResponseError::new("protocol", format!("malformed JSON: {e}")),
                );
                let payload = serde_json::to_string(&resp)?;
                conn.write_frame(&payload)?;
                continue;
            }
        };

        let is_shutdown = matches!(req.command, crate::protocol::Command::Shutdown);

        // Lock the worker only for the duration of this single request
        // This allows other connections to proceed while one connection is being processed
        let mut worker_guard = worker
            .lock()
            .map_err(|e| DaemonError::Protocol(format!("worker mutex poisoned: {e}")))?;

        let mut ctx = DispatchCtx {
            sessions: sessions.clone(),
            runs: runs.clone(),
            worker: &mut *worker_guard,
            started_at_ms,
        };
        let resp = crate::dispatch::dispatch(&mut ctx, req);
        let payload = serde_json::to_string(&resp)?;
        drop(ctx);
        // Worker lock released here
        conn.write_frame(&payload)?;

        if is_shutdown {
            return Ok(());
        }
    }
}
