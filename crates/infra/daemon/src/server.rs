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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::state::EventBus;

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
    /// omenic 自家引擎（orbit）的模型配置：llm_base_url/api_key/model 在
    /// `.oi/config.toml` 齐全时 Some——daemon worker 走 orbit 模式（真
    /// 模型）；None = omp 兼容模式。
    pub orbit_model: Option<adaptor::Model>,
}

impl DaemonConfig {
    /// llm 三件套（base_url/api_key/model）在 `.oi/config.toml` 齐全时
    /// 构建 orbit 模型配置——设置页写该文件即生效。
    fn resolve_orbit_model(cfg: &config::Config) -> Option<adaptor::Model> {
        let base = cfg.llm_base_url.as_ref()?.trim();
        let key = cfg.llm_api_key.as_ref()?.trim();
        let model = cfg.llm_model.as_ref()?.trim();
        if base.is_empty() || key.is_empty() || model.is_empty() {
            return None;
        }
        let mut url = base.trim_end_matches('/').to_string();
        if !url.ends_with("/v1") {
            url.push_str("/v1");
        }
        Some(adaptor::Model {
            api_key: key.to_string(),
            model: model.to_string(),
            base_url: Some(url),
            max_tokens: cfg.llm_max_tokens,
        })
    }

    /// Resolve paths from `Config` and the runtime environment.
    pub fn from_config(cfg: &config::Config) -> Result<Self, DaemonError> {
        let socket_path = Some(cfg.daemon_socket_path()?);
        let session_db_path = Some(cfg.session_db_path()?);
        let omp_path = cfg.omp_path.to_string_lossy().into_owned();
        let orbit_model = Self::resolve_orbit_model(cfg);
        Ok(DaemonConfig {
            socket_path,
            omp_path,
            session_db_path,
            orbit_model,
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
    /// Push-event fan-out table shared by all connections (R2 3.3).
    pub(crate) events: EventBus,
    /// Harness plugin container built by the composition root (C6.5). The
    /// daemon holds it for the process lifetime: dropping the fiber unloads
    /// plugins in reverse registration order, so it must outlive the accept
    /// loop that serves requests against those services.
    pub(crate) _fiber: omenic_composition::Fiber,
    pub(crate) plugins: omenic_composition::PluginRegistry,
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

        // C6.5: build the harness plugin container first. Assembly only reads
        // (InstructionPlugin walks up from cwd for AGENTS.md) and touches
        // nothing outside the fiber, so a duplicate plugin name fails before
        // we take the instance lock or bind the socket — no half-started
        // daemon and no stale lock/socket files to clean up.
        let (fiber, plugins) = Self::assemble_plugins(&cfg)?;

        let lock = InstanceLock::acquire(&socket_path)?;
        let listener = Listener::bind(&socket_path)?;
        let session_db = session::SessionDb::open(&session_db_path)?;
        let run_ledger = RunLedger::open_for_socket(&socket_path)?;

        let session_state = SessionState::new(session_db);
        let worker = Arc::new(Mutex::new(WorkerHandle::new(
            cfg.omp_path.clone(),
            cfg.orbit_model.clone(),
        )));
        let shutdown = Arc::new(AtomicBool::new(false));
        let started_at_ms = now_ms();
        let events = EventBus::new();

        let accept_thread = spawn_accept_loop(AcceptLoopCtx {
            listener,
            worker: Arc::clone(&worker),
            sessions: session_state.clone(),
            runs: run_ledger.clone(),
            shutdown: Arc::clone(&shutdown),
            started_at_ms,
            events: events.clone(),
            next_conn: Arc::new(AtomicU64::new(1)),
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
            events,
            _fiber: fiber,
            plugins,
        })
    }

    /// Assemble the harness plugin container for this daemon.
    ///
    /// The config document is what the composition root reads its knobs from
    /// (`model`, `max_turns`, `cwd`, `system_prompt`). Only `model` has a
    /// daemon-side source today — the orbit model when configured; the rest
    /// stay absent so `assemble` applies its own defaults rather than having
    /// the daemon invent values.
    fn assemble_plugins(
        cfg: &DaemonConfig,
    ) -> Result<
        (
            omenic_composition::Fiber,
            omenic_composition::PluginRegistry,
        ),
        DaemonError,
    > {
        let mut doc = serde_json::Map::new();
        if let Some(model) = cfg.orbit_model.as_ref() {
            doc.insert("model".into(), serde_json::Value::from(model.model.clone()));
        }
        Ok(omenic_composition::assemble(
            serde_json::Value::Object(doc),
            Vec::new(),
        )?)
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

    /// Handle to the push-event bus.  Useful in tests for asserting
    /// subscription state.
    pub fn events(&self) -> &EventBus {
        &self.events
    }

    /// Names of the harness plugins assembled into this daemon, in
    /// registration order (C6.5).  Lets a test assert the container is real
    /// rather than inferring it from behaviour.
    pub fn plugin_names(&self) -> Vec<&str> {
        self.plugins.plugins()
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
    events: EventBus,
    next_conn: Arc<AtomicU64>,
}

fn spawn_accept_loop(ctx: AcceptLoopCtx) -> Result<thread::JoinHandle<()>, DaemonError> {
    let handle = thread::Builder::new()
        .name("omenic-daemon-accept".into())
        .spawn(move || {
            let poll_interval = Duration::from_millis(50);
            let AcceptLoopCtx {
                listener,
                worker,
                sessions,
                runs,
                shutdown,
                started_at_ms,
                events,
                next_conn,
            } = ctx;
            // We poll the shutdown flag between accepts and use a short
            // accept timeout so we don't block forever once shutdown is
            // signalled.
            while !shutdown.load(Ordering::SeqCst) {
                let conn = match listener.accept_timeout(poll_interval) {
                    Ok(Some(c)) => c,
                    Ok(None) => continue, // timeout — re-check shutdown flag
                    Err(_) => break,      // listener closed or poisoned
                };
                let worker = Arc::clone(&worker);
                let sessions = sessions.clone();
                let runs = runs.clone();
                let shutdown = Arc::clone(&shutdown);
                let events = events.clone();
                let conn_id = next_conn.fetch_add(1, Ordering::Relaxed);
                thread::spawn(move || {
                    if let Err(e) = handle_connection(
                        conn,
                        &worker,
                        &sessions,
                        &runs,
                        &shutdown,
                        started_at_ms,
                        events,
                        conn_id,
                    ) {
                        eprintln!("daemon: connection error: {e}");
                    }
                });
            }
        })
        .map_err(DaemonError::Io)?;
    Ok(handle)
}

fn handle_connection(
    conn: Connection,
    worker: &Arc<Mutex<WorkerHandle>>,
    sessions: &SessionState,
    runs: &RunLedger,
    shutdown: &AtomicBool,
    started_at_ms: i64,
    events: EventBus,
    conn_id: u64,
) -> Result<(), DaemonError> {
    // R2 3.3: responses and pushed events share one write channel drained by
    // a dedicated writer thread, so a subscribed connection can receive
    // `EventFrame`s while its read loop blocks on the next request.
    let (mut reader, mut writer) = conn.into_split();
    let (out, inbox) = std::sync::mpsc::channel::<String>();
    let writer_thread = thread::spawn(move || {
        while let Ok(line) = inbox.recv() {
            if writer.write_frame(&line).is_err() {
                break; // peer gone
            }
        }
    });
    let outcome = connection_read_loop(
        &mut reader,
        worker,
        sessions,
        runs,
        shutdown,
        started_at_ms,
        &events,
        conn_id,
        &out,
    );
    // Teardown: stop pushes, wake the writer thread, let queued frames flush.
    events.remove_conn(conn_id);
    drop(out);
    let _ = writer_thread.join();
    outcome
}

#[allow(clippy::too_many_arguments)]
fn connection_read_loop(
    reader: &mut crate::socket::ConnectionReader,
    worker: &Arc<Mutex<WorkerHandle>>,
    sessions: &SessionState,
    runs: &RunLedger,
    shutdown: &AtomicBool,
    started_at_ms: i64,
    events: &EventBus,
    conn_id: u64,
    out: &std::sync::mpsc::Sender<String>,
) -> Result<(), DaemonError> {
    loop {
        let line = match reader.read_frame()? {
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
                if out.send(serde_json::to_string(&resp)?).is_err() {
                    return Ok(()); // writer dead
                }
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
            shutdown,
            events: events.clone(),
            conn_id,
            out: out.clone(),
        };
        let resp = crate::dispatch::dispatch(&mut ctx, req);
        let payload = serde_json::to_string(&resp)?;
        drop(ctx);
        // Worker lock released here
        if out.send(payload).is_err() {
            return Ok(()); // writer dead — connection effectively gone
        }

        if is_shutdown {
            return Ok(());
        }
    }
}
