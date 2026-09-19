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

/// Tool set a fork subagent run receives: the read-only built-in
/// subset (matching dsh subagent-fork-in-process). Write/exec tools
/// stay out of subagent runs by default.
const FORK_SUBAGENT_TOOLS: &[&str] = &["read_file", "grep", "glob"];

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
    /// 会话工作目录：orbit 引擎沿其祖先链查找 `AGENTS.md`（`.oi/config.toml`
    /// 的 `[daemon] cwd`，默认 daemon 启动目录）。写入装配文档并经
    /// `LoopConfig::instruction_cwd` 注入——G6 第一次让指令注入在生产路径
    /// 生效。
    pub cwd: PathBuf,
    /// 每 run 的 LLM 往返上限（`.oi/config.toml` 的 `[daemon] max_turns`）。
    /// 写入装配文档，由 daemon 从 `harness.loop` 服务解析回读。
    pub max_turns: usize,
    /// External MCP servers brought up once at daemon start
    /// (`.oi/config.toml` `[[mcp.servers]]`, wired in by
    /// [`DaemonConfig::from_config`]). Their tools ride the orbit engine
    /// only — see [`Self::orbit_setup`]. Empty = nothing is spawned.
    pub mcp_servers: Vec<config::McpServerConfig>,
    /// Fallback LLM providers tried in order after the primary `[llm]`
    /// provider fails before emitting any content
    /// (`.oi/config.toml` `[[llm.fallbacks]]`). Empty = single-provider
    /// behaviour (`orbit::HttpLlm`, historic path, zero change).
    pub llm_fallbacks: Vec<config::LlmFallbackConfig>,
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
            cwd: cfg.cwd.clone(),
            max_turns: cfg.max_turns.unwrap_or(orbit::DEFAULT_MAX_TURNS),
            mcp_servers: cfg.mcp_servers.clone(),
            llm_fallbacks: cfg.llm_fallbacks.clone(),
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
    /// Harness plugin container built by the composition root (C6.5) and
    /// consumed at start ([`Self::orbit_setup`]). The daemon holds it for the
    /// process lifetime: dropping the fiber unloads plugins in reverse
    /// registration order, so it must outlive the accept loop that serves
    /// requests against those services.
    pub(crate) fiber: omenic_composition::Fiber,
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
        let session_db = session::SessionDb::open(&session_db_path)?;
        let session_state = SessionState::new(session_db);

        // G6 crash repair: a daemon killed mid-run leaves half-open runs in
        // the persisted turn logs. Close them now — after the DB is open and
        // before the socket is bound, so a client never connects to a run
        // the previous process left dangling. Idempotent on a clean log.
        Self::repair_interrupted_runs(&session_state);

        let listener = Listener::bind(&socket_path)?;
        let run_ledger = RunLedger::open_for_socket(&socket_path)?;

        // G6: the container is consumed, not just assembled. The orbit engine
        // gets its tools, compaction policy and turn cap from the services
        // the plugins provided (falling back to each family's own default
        // when a service is absent). orbit stays the production loop; the
        // container supplies config and policy, the worker bridges.
        //
        // B1/T3: MCP rides the orbit engine — the configured servers are
        // spawned and handshaken exactly once, inside `orbit_setup`. In
        // omp-compat mode (`orbit_model` is None) this never runs and MCP
        // tools don't exist for the daemon worker; omp peers have their own
        // external-tool registration path (task CLI).
        let orbit_setup = match cfg.orbit_model.as_ref() {
            Some(model) => Some(Self::orbit_setup(&fiber, model, &cfg)?),
            None => None,
        };

        let worker = Arc::new(Mutex::new(WorkerHandle::new(
            cfg.omp_path.clone(),
            orbit_setup,
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
            fiber,
            plugins,
        })
    }

    /// Resolve the orbit engine's setup out of the assembled container: the
    /// tool catalog (`harness.tools`), the compaction policy
    /// (`harness.compaction`), and the turn cap (`harness.loop`, itself built
    /// from the config document — see [`Self::assemble_plugins`]).
    ///
    /// Every resolution degrades to the family's own default rather than
    /// failing the daemon start: a container that doesn't provide a service
    /// still yields a working engine on the documented defaults.
    ///
    /// Errors only from MCP bring-up: a server with
    /// `fail_on_startup_error = true` that fails to start aborts the daemon
    /// start loudly (B1/T2 semantics honored at the daemon boundary).
    fn orbit_setup(
        fiber: &omenic_composition::Fiber,
        model: &adaptor::Model,
        cfg: &DaemonConfig,
    ) -> Result<rpc::worker::OrbitSetup, DaemonError> {
        let catalog = fiber
            .resolve::<omenic_harness_tools::ToolCatalog>("harness.tools")
            .unwrap_or_else(|| std::sync::Arc::new(omenic_harness_tools::default_catalog()));
        let compaction = fiber
            .resolve::<omenic_harness_compaction::CharBudgetPolicy>("harness.compaction")
            .unwrap_or_else(|| {
                std::sync::Arc::new(omenic_harness_compaction::CharBudgetPolicy::default())
            });
        let max_turns = fiber
            .resolve::<omenic_harness_runtime::LoopEngine>("harness.loop")
            .map(|engine| engine.max_turns)
            .unwrap_or(cfg.max_turns);
        // The fork subagent shares the engine's turn budget (documented
        // choice: one knob, no new config surface in this task).
        let subagent_max_turns = max_turns;
        // B2b1 — waterfall LLM fallback: with `[[llm.fallbacks]]`
        // configured the backend becomes `WaterfallLlm` (primary +
        // fallbacks, per-provider retry inside, no switch after content
        // leaks). Empty list keeps the historic `HttpLlm` path verbatim —
        // zero behaviour change for single-provider configs.
        let backend: std::sync::Arc<dyn orbit::LlmBackend + Send + Sync> =
            if cfg.llm_fallbacks.is_empty() {
                std::sync::Arc::new(orbit::HttpLlm)
            } else {
                let primary = orbit::LlmProvider {
                    api_key: model.api_key.clone(),
                    model: model.model.clone(),
                    base_url: model.base_url.clone(),
                    max_tokens: model.max_tokens,
                };
                let fallbacks = cfg
                    .llm_fallbacks
                    .iter()
                    .enumerate()
                    .filter_map(|(i, f)| {
                        // A model-less fallback row can never be dialed (the
                        // waterfall switches by model id) — skip it, but say
                        // so: a typo'd row silently dropping out of the
                        // waterfall otherwise looks identical to one that is
                        // simply never reached.
                        let fallback_model = f
                            .model
                            .as_deref()
                            .map(str::trim)
                            .filter(|m| !m.is_empty())
                            .map(str::to_string);
                        if fallback_model.is_none() {
                            eprintln!("warn: [[llm.fallbacks]] entry #{i} has no model; skipped");
                        }
                        fallback_model.map(|fallback_model| orbit::LlmProvider {
                            api_key: f
                                .api_key
                                .clone()
                                .or_else(|| Some(model.api_key.clone()))
                                .unwrap_or_default(),
                            model: fallback_model,
                            base_url: f.base_url.clone().or_else(|| model.base_url.clone()),
                            max_tokens: f.max_tokens,
                        })
                    })
                    .collect();
                std::sync::Arc::new(orbit::WaterfallLlm::new(primary, fallbacks))
            };
        // Subagent seam: resolve the container's SubagentRuntimeService and
        // register the in-process fork provider into it (mutating the
        // container's shared service — a documented side effect of
        // daemon startup), so the model-facing `subagent` tool resolves a
        // real provider. Resolution degrades to a fresh service when the
        // container lacks the key (same style as the catalog/compaction
        // fallbacks above).
        let subagents = fiber
            .resolve::<omenic_harness_subagent::SubagentRuntimeService>("harness.subagents")
            .unwrap_or_else(|| {
                std::sync::Arc::new(omenic_harness_subagent::SubagentRuntimeService::default())
            });
        let fork_tools = std::sync::Arc::new(omenic_harness_tools::filter_builtin_tools(
            FORK_SUBAGENT_TOOLS,
        ));
        subagents.register(
            "fork",
            std::sync::Arc::new(omenic_harness_subagent::ForkProvider::new(
                std::sync::Arc::clone(&backend),
                model.clone(),
                fork_tools,
                subagent_max_turns,
            )),
        );
        // B1/T3 — MCP bring-up: spawn every configured server exactly once
        // per daemon start and hand their tools to the engine. Default
        // policy is best-effort: a server that fails to start/handshake
        // contributes zero tools and is skipped (logged by the mcp crate) —
        // one broken entry in the user's config must not take down the
        // daemon. Each handshake is bounded by the mcp crate's 30s
        // MCP_TIMEOUT, so worst-case bring-up latency is
        // N servers × 30s: bounded, a hung server cannot block `Daemon::start`
        // forever.
        //
        // The tools cross into the engine through `OrbitConfig::mcp_tools`
        // (agent-domain `tools::Tool`), deliberately NOT through the harness
        // `ToolCatalog`: the two tool traits are separate on purpose (C6) —
        // infra/daemon may bridge them, the catalog must not be polluted.
        let signal = std::sync::atomic::AtomicBool::new(false);
        let mcp_tools = Self::mcp_tools(cfg, &signal)?;
        Ok(rpc::worker::OrbitSetup {
            model: model.clone(),
            backend,
            config: rpc::worker::OrbitConfig {
                cwd: Some(std::sync::Arc::from(cfg.cwd.clone())),
                max_turns,
                compaction,
                catalog,
                mcp_tools,
            },
            // Seam intent: the daemon registers the in-process fork provider
            // above; `providers` carries the (name, tool allow-list) intent
            // forward for Phase 4 out-of-process providers.
            providers: vec![(
                "fork".into(),
                FORK_SUBAGENT_TOOLS.iter().map(|s| s.to_string()).collect(),
            )],
        })
    }

    /// Spawn the configured MCP servers and collect their tools as shared
    /// handles. `external_tools_from_mcp` yields owned `Box<dyn Tool>`s;
    /// `Arc::from` re-owns each box so the list can be shared across engine
    /// respawns (`OrbitSetup` is cloned per spawn, and a `Box` cannot be).
    ///
    /// `Err` only when a server set `fail_on_startup_error = true` and its
    /// startup failed. The mcp crate wraps every per-server failure with the
    /// config entry's name (`server \`{name}\`: ...`), so this prefix stays
    /// short and names the offender exactly.
    fn mcp_tools(
        cfg: &DaemonConfig,
        signal: &std::sync::atomic::AtomicBool,
    ) -> Result<std::sync::Arc<Vec<std::sync::Arc<dyn tools::Tool>>>, DaemonError> {
        let brought = mcp::external_tools_from_mcp(&cfg.mcp_servers, signal)
            .map_err(|e| DaemonError::Protocol(format!("MCP bring-up failed: {e}")))?;
        Ok(std::sync::Arc::new(
            brought.into_iter().map(std::sync::Arc::from).collect(),
        ))
    }

    /// Append synthetic `TurnEnd { aborted }` records for every run a prior
    /// process left open. Best-effort: a storage failure logs and skips that
    /// session rather than aborting the daemon start.
    fn repair_interrupted_runs(sessions: &SessionState) {
        let ids = match sessions.session_ids() {
            Ok(ids) => ids,
            Err(e) => {
                eprintln!("daemon: turn-log scan failed, skipping crash repair: {e}");
                return;
            }
        };
        for id in &ids {
            let Ok(records) = sessions.load_turn_log(id) else {
                continue;
            };
            let closers = session::interrupted_run_closers(&records);
            if closers.is_empty() {
                continue;
            }
            match sessions.append_turn_log(id, &closers) {
                Ok(()) => eprintln!(
                    "daemon: crash repair closed {} interrupted run(s) in session {id}",
                    closers.len()
                ),
                Err(e) => eprintln!("daemon: crash repair of session {id} failed: {e}"),
            }
        }
    }

    /// Assemble the harness plugin container for this daemon.
    ///
    /// The config document is what the composition root reads its knobs from
    /// (`model`, `max_turns`, `cwd`, `system_prompt`). `model` comes from the
    /// orbit credentials when configured; `cwd` and `max_turns` come from
    /// `[daemon]` in `.oi/config.toml`. The document is the single source the
    /// container consumes — the daemon then reads the assembled services back
    /// out (see [`Self::orbit_setup`]), so a knob flows document → service →
    /// engine rather than being short-circuited.
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
        doc.insert(
            "cwd".into(),
            serde_json::Value::from(cfg.cwd.to_string_lossy().into_owned()),
        );
        doc.insert(
            "max_turns".into(),
            serde_json::Value::from(cfg.max_turns as u64),
        );
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

    /// The assembled plugin container (C6.5): every service the plugins
    /// provided at start is resolvable from here for the daemon's lifetime.
    /// G6: the daemon resolves the orbit engine's tools / compaction policy /
    /// turn cap out of this rather than hardcoding them.
    pub fn fiber(&self) -> &omenic_composition::Fiber {
        &self.fiber
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
            worker: &mut worker_guard,
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
