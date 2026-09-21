//! Command dispatch.
//!
//! Translates a [`Request`] into a [`Response`] by routing to either the
//! [`SessionState`] handle or the [`WorkerHandle`].  Pure function over
//! `&mut WorkerHandle` so the caller (server accept loop) can serialize
//! worker access across concurrent connections.
//!
//! ponytail: dispatch is split out so the server module can stay tiny.  All
//! command logic lives here, all protocol concerns live in `protocol.rs`,
//! and the worker handle is the only piece that knows about `rpc::Worker`.

use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use rpc::worker::WorkerEvent;
use serde_json::{Value, json};
use session::{SessionMessage, SessionRole, TurnRecord};

use crate::protocol::{Command, EventFrame, Request, Response, ResponseError};
use crate::state::{EventBus, RunLedger, SessionState, require_str, require_u32};

/// Topic fed by the `rpc` worker's push event stream (R2 3.3).
pub const WORKER_TOPIC: &str = "worker";

/// How many persisted messages a resume replays into the worker's live
/// context on a `worker.prompt` that carries a `session_id` (B2a).  A hard
/// cap: the tail of a long session can be tens of thousands of rows, and the
/// orbit context only needs a working window, not the whole history.  50
/// recent messages is enough to keep the turn coherent without forcing a
/// respawned engine to swallow an unbounded body.
const RESUME_CONTEXT_LIMIT: u32 = 50;

/// Default `task.list` page size when the client sends no `limit`.  The
/// kanban board renders one screen; anything past the 50 most recently
/// touched tasks is stale backlog the UI can page in later.
const TASK_LIST_DEFAULT_LIMIT: u32 = 50;

/// Shared worker handle.  The dispatch layer takes `&mut` so concurrent
/// connections are serialized by the server's mutex.
pub struct WorkerHandle {
    inner: Option<rpc::worker::Worker>,
    omp_path: String,
    /// Whether the forwarder thread feeding [`WORKER_TOPIC`] is alive.
    /// Cleared by the forwarder itself when the worker's event channel
    /// closes (worker died or was reset), so the next subscribe respawns it.
    pump_active: Arc<AtomicBool>,
    /// The run served by the prompt currently in flight (G7-B run
    /// attribution).  The event pump reads this to stamp every
    /// [`EventFrame`] it broadcasts, so subscribers can route events to the
    /// run that owns them rather than to the session that sent most
    /// recently.  `None` while no attributed prompt is running.
    active_run: Arc<std::sync::RwLock<Option<String>>>,
    /// orbit 模式构造包（模型 + 后端 + 容器解析出的 OrbitConfig）；
    /// None = omp 兼容模式。daemon 在 start 时从装配容器解析一次。
    orbit_setup: Option<rpc::worker::OrbitSetup>,
}

impl WorkerHandle {
    pub fn new(omp_path: impl Into<String>, orbit_setup: Option<rpc::worker::OrbitSetup>) -> Self {
        WorkerHandle {
            inner: None,
            omp_path: omp_path.into(),
            pump_active: Arc::new(AtomicBool::new(false)),
            active_run: Arc::new(std::sync::RwLock::new(None)),
            orbit_setup,
        }
    }

    /// Record the run served by the prompt about to be sent (G7-B).  The
    /// event pump stamps this onto every frame it pushes while the run is
    /// active.  The slot is *sticky*: `prompt` returns as soon as the worker
    /// acks, but the turn's events keep flowing afterwards (the pump owns
    /// the read loop and fans them out after the response frame is matched
    /// — in orbit mode the whole turn is async), so the run is only replaced
    /// by the next attributed prompt, never cleared on prompt return.
    /// An empty `run_id` (legacy prompt without attribution) clears the
    /// slot so a finished run cannot own a later, unattributed turn.
    pub fn set_active_run(&self, run_id: &str) {
        let mut guard = self.active_run.write().unwrap_or_else(|e| e.into_inner());
        *guard = (!run_id.is_empty()).then(|| run_id.to_string());
    }

    /// Forget any active run (worker reset: a respawned worker owes the
    /// previous run nothing).
    pub fn clear_active_run(&self) {
        let mut guard = self.active_run.write().unwrap_or_else(|e| e.into_inner());
        *guard = None;
    }

    /// Whether this handle drives the omenic orbit engine in-process (`Some`
    /// orbit setup) instead of an external omp worker (G8).
    ///
    /// The split changes run bookkeeping fundamentally: an omp `prompt`
    /// blocks for the whole turn and its return value *is* the turn's
    /// terminal state, so `dispatch` closes the run synchronously.  An orbit
    /// `prompt` only acknowledges the message was queued — the turn runs on
    /// the engine's serial thread and ends later, when the event pump
    /// forwards `WorkerEvent::AgentEnd`.  Closing on the ack would make every
    /// run look finished the instant it started.
    pub fn is_orbit(&self) -> bool {
        self.orbit_setup.is_some()
    }

    /// PID of the underlying omp worker (0 if not yet spawned).
    pub fn child_pid(&self) -> u32 {
        self.inner.as_ref().map(|w| w.child_pid()).unwrap_or(0)
    }

    /// Lazy-spawn the worker if it isn't running yet, and register the
    /// daemon-owned `session_query` tool so the agent sees exactly one
    /// daemon-backed entry to the session store.
    fn ensure_started(&mut self) -> Result<(), Response> {
        if self.inner.is_none() {
            let mut w = rpc::worker::Worker::new(&self.omp_path, self.orbit_setup.clone())
                .map_err(|e| {
                    Response::err(
                        None,
                        ResponseError::new("worker_spawn_failed", e.to_string()),
                    )
                })?;
            // Register session_query as the single daemon-backed tool. A
            // registration failure is non-fatal: omp might not implement
            // the call yet, and we don't want tool negotiation to take
            // the worker down. Log via stderr so operators see it.
            let def = crate::session_query::session_query_def();
            if let Err(e) = w.register_external_tools(vec![def]) {
                eprintln!("daemon: external tool registration failed: {e}");
            }
            self.inner = Some(w);
        }
        Ok(())
    }

    /// Drop the worker entirely; next call lazy-respawns.  The forwarder
    /// thread observes the closed event channel and clears `pump_active`.
    pub fn reset(&mut self) {
        self.clear_active_run();
        self.inner = None;
    }

    /// Replay persisted session history into the live worker before a prompt
    /// (B2a resume).  Delegates to the inner [`rpc::worker::Worker`], which is
    /// a no-op returning 0 in omp mode (there is no in-process engine context
    /// to rebuild) and, in orbit mode, appends the user/assistant rows to the
    /// engine's context — deduped per session id inside the engine, so the
    /// daemon may safely call this on every prompt that carries a
    /// `session_id` without doubling the history.
    ///
    /// The worker must be live first: callers invoke
    /// [`Self::ensure_started`] before this.  A no-op `Ok(0)` on a handle
    /// that was just `reset()` keeps the resume contract total.
    pub fn resume_session(
        &mut self,
        session_id: &str,
        messages: &[SessionMessage],
    ) -> Result<(), Response> {
        if self.inner.is_none() {
            return Ok(());
        }
        let w = self.inner.as_mut().expect("inner checked");
        match w.resume_session(session_id, messages) {
            Ok(_) => Ok(()),
            Err(e) => Err(Response::err(
                None,
                ResponseError::new("worker_resume_failed", e.to_string()),
            )),
        }
    }

    /// Start the worker-side event pump once (R2 3.3): `rpc::Worker::
    /// subscribe` hands the wire read loop to a pump thread; this daemon
    /// side forwarder drains that receiver and broadcasts [`EventFrame`]
    /// lines to every [`WORKER_TOPIC`] subscriber on the [`EventBus`].
    /// Serialized against every other worker use by the server's mutex.
    ///
    /// In orbit mode this thread is also the run's undertaker (G8): a prompt
    /// only acknowledges that the message was queued, so the ledger run and
    /// the session's turn log stay open until the pump forwards the turn's
    /// [`WorkerEvent::AgentEnd`].  `sessions` / `runs` are shared in for that
    /// close.
    pub fn ensure_event_pump(
        &mut self,
        events: &EventBus,
        sessions: &SessionState,
        runs: &RunLedger,
    ) -> Result<(), Response> {
        if self.pump_active.load(Ordering::SeqCst) {
            return Ok(());
        }
        self.ensure_started()?;
        let w = self.inner.as_mut().expect("ensured");
        let rx = w.subscribe(WORKER_TOPIC);
        let active = Arc::clone(&self.pump_active);
        let active_run = Arc::clone(&self.active_run);
        let bus = events.clone();
        let sessions = sessions.clone();
        let runs = runs.clone();
        // Orbit runs end here, omp-compat runs end in `dispatch` when the
        // blocking prompt returns — the pump must not second-guess that
        // close (it would append a second TurnEnd per turn).
        let is_orbit = self.is_orbit();
        active.store(true, Ordering::SeqCst);
        std::thread::spawn(move || {
            // Ends when the worker dies (rpc pump clears its subscriber
            // table).  Events with no subscribers are dropped by broadcast.
            while let Ok(event) = rx.recv() {
                let Ok(payload) = serde_json::to_value(&event) else {
                    continue;
                };
                let mut frame = EventFrame::new(WORKER_TOPIC, payload);
                // G7-B: attribute the frame to the run the in-flight prompt
                // declared, so subscribers route it to the owning run.
                // Read-only here; a prompt on another connection may be
                // setting the slot concurrently (the server serializes
                // prompts by the worker mutex, but the pump keeps draining
                // this run's events after that prompt returned).
                //
                // The run is snapshotted once and reused below: the finish
                // must close exactly the run this frame was attributed to,
                // not whatever a concurrent prompt left in the slot later.
                let attributed = active_run.read().unwrap_or_else(|e| e.into_inner()).clone();
                if let Some(run) = attributed.as_deref() {
                    frame = frame.with_run_id(run);
                }
                // G8: an orbit turn's terminal event is AgentEnd on this
                // stream, not the prompt's ack. Close the ledger run and the
                // turn log here, before the frame goes out, so a subscriber
                // reading the end frame sees a run that is already closed.
                // Then release the sticky slot (compare-and-set — see
                // [`try_clear_active_run`]) so later unattributed events do
                // not keep stamping a run that already ended.
                if is_orbit
                    && let WorkerEvent::AgentEnd { stop_reason } = &event
                    && let Some(run) = attributed.as_deref()
                {
                    close_run_on_agent_end(&runs, &sessions, run, stop_reason);
                    try_clear_active_run(&active_run, run);
                }
                let Ok(line) = serde_json::to_string(&frame) else {
                    continue;
                };
                bus.broadcast(WORKER_TOPIC, &line);
            }
            active.store(false, Ordering::SeqCst);
        });
        Ok(())
    }
}

/// Finish a run and append its `TurnEnd` when the orbit engine signals the
/// end of the turn (G8).  Idempotent: a run already closed (a repeated
/// `AgentEnd` for the same turn) is left alone, so the turn log keeps exactly
/// one `TurnEnd` per `TurnStart` — that start/end balance is exactly what
/// [`session::interrupted_run_closers`] walks at startup.
fn close_run_on_agent_end(
    runs: &RunLedger,
    sessions: &SessionState,
    run_id: &str,
    stop_reason: &str,
) {
    // Snapshot before finishing: the session id has to survive a concurrent
    // close of the same run, and a run that is already done is not ours to
    // close.
    let Some(record) = runs.get(run_id) else {
        return;
    };
    if record.finished_at_ms.is_some() {
        return;
    }
    let status = crate::state::agent_end_status(stop_reason);
    let ts_ms = crate::state::now_ms();
    if let Err(e) = runs.finish(run_id, ts_ms, status) {
        eprintln!("daemon: AgentEnd close failed for run {run_id}: {e}");
    }
    record_turn(
        sessions,
        &record.session_id,
        run_id,
        TurnRecord::TurnEnd {
            run_id: run_id.to_string(),
            ts_ms,
            status: status.into(),
        },
    );
}

/// Clear the sticky active-run slot only while it still holds `expected`
/// (G8).
///
/// The pump reads the slot, finishes that run, then clears — but between the
/// read and the write a prompt on another connection may have swapped the
/// slot to a brand-new run `r2`: the server serializes dispatch by the
/// worker mutex, yet the pump keeps draining the finished run's trailing
/// events after its own prompt returned, and the next prompt's
/// [`WorkerHandle::set_active_run`] is a separate lock acquisition that can
/// land in that gap.  Blinding the slot to `None` would orphan `r2` — its
/// events would lose their run attribution and *its* `AgentEnd` would find
/// an empty slot and never close the run, which is precisely the
/// half-open-run bug this change fixes.  Comparing `expected` under the
/// write lock turns the clear into a compare-and-swap: the slot is dropped
/// only if nobody replaced it in between, and `r2` keeps its owner.
fn try_clear_active_run(slot: &RwLock<Option<String>>, expected: &str) {
    let mut guard = slot.write().unwrap_or_else(|e| e.into_inner());
    if guard.as_deref() == Some(expected) {
        *guard = None;
    }
}

/// Per-connection dispatch context.  Carries the shared state + the worker
/// handle.  The server holds the worker handle behind a mutex so concurrent
/// connections don't trample each other's in-flight RPC frames.
pub struct DispatchCtx<'a> {
    pub sessions: SessionState,
    pub runs: RunLedger,
    pub worker: &'a mut WorkerHandle,
    pub started_at_ms: i64,
    pub shutdown: &'a std::sync::atomic::AtomicBool,
    /// Push-event fan-out table (R2 3.3).
    pub events: EventBus,
    /// Identity of the connection being dispatched, for subscription
    /// teardown on disconnect.
    pub conn_id: u64,
    /// This connection's write channel; subscriptions clone it.
    pub out: Sender<String>,
    /// Where the CLI writes `tasks.jsonl` (the `.oi` data dir).
    /// `task.list` builds a `task::Store` here per request — the store
    /// is a stateless `PathBuf` wrapper, so there is nothing to cache.
    pub task_data_dir: std::path::PathBuf,
    /// Pending user questions (plan-mode review and friends).
    pub questions: std::sync::Arc<crate::questions::QuestionBroker>,
    /// Plan-mode state: `/plan` commands flip it between turns.
    pub plan_mode: omenic_harness_plan_mode::PlanModeRuntime,
}

/// Dispatch a single request.  Always returns a `Response`; the caller just
pub fn dispatch(ctx: &mut DispatchCtx<'_>, req: Request) -> Response {
    let id = req.id.as_deref();
    match req.command {
        // ---------------- Daemon-level ----------------
        Command::Ping => Response::ok(id, json!({ "pong": true })),

        Command::Shutdown => {
            ctx.shutdown
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Response::ok(id, json!({ "shutting_down": true }))
        }

        Command::Info => Response::ok(
            id,
            json!({
                "pid": std::process::id(),
                "started_at_ms": ctx.started_at_ms,
                "uptime_ms": crate::state::now_ms() - ctx.started_at_ms,
                "worker_pid": ctx.worker.child_pid(),
            }),
        ),

        // ---------------- Session ----------------
        Command::SessionCreate => {
            let sid = match require_str(&req.params, "session_id") {
                Ok(s) => s,
                Err(m) => return Response::err(id, ResponseError::new("protocol", m)),
            };
            let title = match require_str(&req.params, "title") {
                Ok(s) => s,
                Err(m) => return Response::err(id, ResponseError::new("protocol", m)),
            };
            // G7-A1: optional lineage parent. Absent / null / blank → root,
            // so a client that predates parent_id keeps working untouched.
            let parent_id = req
                .params
                .get("parent_id")
                .and_then(Value::as_str)
                .filter(|p| !p.trim().is_empty());
            match ctx
                .sessions
                .ensure_session_with_parent(sid, title, parent_id)
            {
                Ok(row) => match serde_json::to_value(&row) {
                    Ok(v) => Response::ok(id, v),
                    Err(e) => Response::err(
                        id,
                        ResponseError::new("internal", format!("serialize: {e}")),
                    ),
                },
                Err(e) => session_error_response(id, "session.create", e),
            }
        }

        Command::SessionUpdateTitle => {
            let sid = match require_str(&req.params, "session_id") {
                Ok(s) => s,
                Err(m) => return Response::err(id, ResponseError::new("protocol", m)),
            };
            let title = match require_str(&req.params, "title") {
                Ok(s) => s,
                Err(m) => return Response::err(id, ResponseError::new("protocol", m)),
            };
            // UPDATE-only rename: a missing session is a `database_missing`
            // error response (never an insert), same translation as
            // `session.create`'s store errors.
            match ctx.sessions.update_title(sid, title) {
                Ok(row) => match serde_json::to_value(&row) {
                    Ok(v) => Response::ok(id, v),
                    Err(e) => Response::err(
                        id,
                        ResponseError::new("internal", format!("serialize: {e}")),
                    ),
                },
                Err(e) => session_error_response(id, "session.update_title", e),
            }
        }

        Command::SessionGet => {
            let sid = match require_str(&req.params, "session_id") {
                Ok(s) => s,
                Err(m) => return Response::err(id, ResponseError::new("protocol", m)),
            };
            match ctx.sessions.session(sid) {
                Ok(Some(row)) => match serde_json::to_value(&row) {
                    Ok(v) => Response::ok(id, v),
                    Err(e) => Response::err(
                        id,
                        ResponseError::new("internal", format!("serialize: {e}")),
                    ),
                },
                Ok(None) => Response::ok(id, Value::Null),
                Err(e) => session_error_response(id, "session.get", e),
            }
        }

        Command::SessionList => {
            let q = match require_str(&req.params, "query") {
                Ok(s) => s,
                Err(m) => return Response::err(id, ResponseError::new("protocol", m)),
            };
            let limit = match require_u32(&req.params, "limit") {
                Ok(n) => n,
                Err(m) => return Response::err(id, ResponseError::new("protocol", m)),
            };
            match ctx.sessions.list_sessions(q, limit) {
                Ok(rows) => match serde_json::to_value(&rows) {
                    Ok(v) => Response::ok(id, v),
                    Err(e) => Response::err(
                        id,
                        ResponseError::new("internal", format!("serialize: {e}")),
                    ),
                },
                Err(e) => session_error_response(id, "session.list", e),
            }
        }

        Command::SessionDelete => {
            let sid = match require_str(&req.params, "session_id") {
                Ok(s) => s,
                Err(m) => return Response::err(id, ResponseError::new("protocol", m)),
            };
            match ctx.sessions.delete_session(sid) {
                Ok(deleted) => Response::ok(id, json!({ "deleted": deleted })),
                Err(e) => session_error_response(id, "session.delete", e),
            }
        }

        // ---------------- Run ----------------
        Command::RunList => {
            let limit = match require_u32(&req.params, "limit") {
                Ok(n) => n,
                Err(m) => return Response::err(id, ResponseError::new("protocol", m)),
            };
            let mut runs = ctx.runs.list();
            let keep_from = runs.len().saturating_sub(limit as usize);
            runs.drain(..keep_from);
            match serde_json::to_value(&runs) {
                Ok(v) => Response::ok(id, v),
                Err(e) => Response::err(
                    id,
                    ResponseError::new("internal", format!("serialize: {e}")),
                ),
            }
        }

        // ---------------- Task ----------------
        Command::TaskList => {
            // `limit` is optional (run.list's is required): the board
            // sends a page size only once it paginates, a bare `{}` is
            // the default page.  A missing / non-number falls back too
            // — a stale client must not break a newer daemon.
            let limit = req
                .params
                .get("limit")
                .and_then(Value::as_u64)
                .map(|n| n.min(u32::MAX as u64) as u32)
                .unwrap_or(TASK_LIST_DEFAULT_LIMIT);
            let store = task::store::Store::new(&ctx.task_data_dir);
            let mut tasks = match store.load_all() {
                Ok(t) => t,
                Err(e) => {
                    return Response::err(
                        id,
                        ResponseError::new("internal", format!("task store: {e}")),
                    );
                }
            };
            // `updated_at` is ISO-8601 UTC at second precision, so
            // lexical order *is* chronological.  `sort_by` is stable,
            // so same-second tasks keep `load_all`'s id order instead
            // of reshuffling between requests.
            tasks.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
            tasks.truncate(limit as usize);
            match serde_json::to_value(&tasks) {
                Ok(v) => Response::ok(id, v),
                Err(e) => Response::err(
                    id,
                    ResponseError::new("internal", format!("serialize: {e}")),
                ),
            }
        }

        // ---------------- Todo ----------------
        Command::TodoList => {
            // Same contract as `task.list`: `limit` optional, a stale
            // client's missing / non-number value falls back to the
            // default page rather than failing.
            let limit = req
                .params
                .get("limit")
                .and_then(Value::as_u64)
                .map(|n| n.min(u32::MAX as u64) as u32)
                .unwrap_or(TASK_LIST_DEFAULT_LIMIT);
            let store = task::store::Store::new(&ctx.task_data_dir);
            let mut todos = match store.load_todos() {
                Ok(t) => t,
                Err(e) => {
                    return Response::err(
                        id,
                        ResponseError::new("internal", format!("todo store: {e}")),
                    );
                }
            };
            // ISO-8601 UTC at second precision: lexical order *is*
            // chronological, and `sort_by` is stable, so same-second
            // todos keep the store's id order.
            todos.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
            todos.truncate(limit as usize);
            match serde_json::to_value(&todos) {
                Ok(v) => Response::ok(id, v),
                Err(e) => Response::err(
                    id,
                    ResponseError::new("internal", format!("serialize: {e}")),
                ),
            }
        }

        // ---------------- Goal ----------------
        Command::GoalList => {
            // Same contract as `task.list` / `todo.list` (shared page
            // size constant — todos, goals and tasks page alike).
            let limit = req
                .params
                .get("limit")
                .and_then(Value::as_u64)
                .map(|n| n.min(u32::MAX as u64) as u32)
                .unwrap_or(TASK_LIST_DEFAULT_LIMIT);
            let store = task::store::Store::new(&ctx.task_data_dir);
            let mut goals = match store.load_goals() {
                Ok(g) => g,
                Err(e) => {
                    return Response::err(
                        id,
                        ResponseError::new("internal", format!("goal store: {e}")),
                    );
                }
            };
            goals.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
            goals.truncate(limit as usize);
            match serde_json::to_value(&goals) {
                Ok(v) => Response::ok(id, v),
                Err(e) => Response::err(
                    id,
                    ResponseError::new("internal", format!("serialize: {e}")),
                ),
            }
        }

        // ---------------- Stats (G5) ----------------
        Command::StatsSummary => {
            // `range` is optional: absent / unknown falls back to "24h"
            // (see `StatsRange::parse`), so a bare `{}` is a valid call.
            let range = req
                .params
                .get("range")
                .and_then(Value::as_str)
                .unwrap_or("24h");
            let summary = ctx.runs.stats(range, crate::state::now_ms());
            match serde_json::to_value(&summary) {
                Ok(v) => Response::ok(id, v),
                Err(e) => Response::err(
                    id,
                    ResponseError::new("internal", format!("serialize: {e}")),
                ),
            }
        }

        Command::SessionAppend => {
            let sid = match require_str(&req.params, "session_id") {
                Ok(s) => s,
                Err(m) => return Response::err(id, ResponseError::new("protocol", m)),
            };
            let role_str = match require_str(&req.params, "role") {
                Ok(s) => s,
                Err(m) => return Response::err(id, ResponseError::new("protocol", m)),
            };
            let text = match require_str(&req.params, "text") {
                Ok(s) => s,
                Err(m) => return Response::err(id, ResponseError::new("protocol", m)),
            };
            let role = match SessionRole::parse(role_str) {
                Ok(r) => r,
                Err(_) => {
                    return Response::err(
                        id,
                        ResponseError::new("protocol", format!("unknown role `{role_str}`")),
                    );
                }
            };
            match ctx.sessions.append_message(sid, role, text) {
                Ok((seq, ts)) => Response::ok(id, json!({ "seq": seq, "created_at_ms": ts })),
                Err(e) => session_error_response(id, "session.append", e),
            }
        }

        Command::SessionLoadMessages => {
            let sid = match require_str(&req.params, "session_id") {
                Ok(s) => s,
                Err(m) => return Response::err(id, ResponseError::new("protocol", m)),
            };
            let limit = match require_u32(&req.params, "limit") {
                Ok(n) => n,
                Err(m) => return Response::err(id, ResponseError::new("protocol", m)),
            };
            match ctx.sessions.load_messages(sid, limit) {
                Ok(rows) => match serde_json::to_value(&rows) {
                    Ok(v) => Response::ok(id, v),
                    Err(e) => Response::err(
                        id,
                        ResponseError::new("internal", format!("serialize: {e}")),
                    ),
                },
                Err(e) => session_error_response(id, "session.load_messages", e),
            }
        }

        Command::SessionSearch => {
            let q = match require_str(&req.params, "query") {
                Ok(s) => s,
                Err(m) => return Response::err(id, ResponseError::new("protocol", m)),
            };
            let scope = req
                .params
                .get("scope")
                .and_then(|value| value.get("id"))
                .and_then(Value::as_str);
            let limit = match require_u32(&req.params, "limit") {
                Ok(n) => n,
                Err(m) => return Response::err(id, ResponseError::new("protocol", m)),
            };
            match ctx.sessions.search_messages(q, scope, limit) {
                Ok(rows) => match serde_json::to_value(&rows) {
                    Ok(v) => Response::ok(id, v),
                    Err(e) => Response::err(
                        id,
                        ResponseError::new("internal", format!("serialize: {e}")),
                    ),
                },
                Err(e) => session_error_response(id, "session.search", e),
            }
        }

        Command::SessionReadFromCursor => {
            let cursor = req
                .params
                .get("cursor")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let (runs, next_cursor) = ctx.runs.read_from_cursor(cursor);
            Response::ok(id, json!({ "runs": runs, "cursor": next_cursor }))
        }

        // ---------------- Worker ----------------
        Command::WorkerPing => {
            if let Err(e) = ctx.worker.ensure_started() {
                return e;
            }
            let w = ctx.worker.inner.as_mut().expect("ensured");
            match w.ping() {
                Ok(()) => Response::ok(id, json!({ "pong": true })),
                Err(e) => {
                    Response::err(id, ResponseError::new("worker_ping_failed", e.to_string()))
                }
            }
        }

        Command::WorkerPrompt => {
            let msg = match require_str(&req.params, "message") {
                Ok(s) => s.to_string(),
                Err(m) => return Response::err(id, ResponseError::new("protocol", m)),
            };
            // Turn boundary: land a change parked by an approved exit
            // (`prepare_approved_exit` commits pending=false without
            // touching active). The next prompt is that boundary — without
            // this the exit never applies and plan mode stays active
            // forever. Best-effort: a poisoned mutex cannot be fixed here.
            if let Ok(Some(mutation)) = ctx.plan_mode.prepare_boundary() {
                let _ = mutation.commit();
            }
            // `/plan` family: flip plan-mode state between turns instead of
            // prompting. A message argument enters plan mode first and then
            // falls through, so the text still reaches the model.
            let mut prompt_text: Option<String> = None;
            if let Some(parsed) = ctx.plan_mode.parse_command(&msg) {
                let command = match parsed {
                    Ok(c) => c,
                    Err(e) => {
                        return Response::err(
                            id,
                            ResponseError::new("plan_command_invalid", e.to_string()),
                        );
                    }
                };
                let target = match command {
                    omenic_harness_plan_mode::PlanModeCommand::Enter { .. } => true,
                    omenic_harness_plan_mode::PlanModeCommand::Off => false,
                };
                match command {
                    omenic_harness_plan_mode::PlanModeCommand::Enter {
                        message: Some(text),
                    } => prompt_text = Some(text),
                    _ => {}
                }
                match ctx.plan_mode.prepare_set(target) {
                    Ok(Some(mutation)) => {
                        if let Err(e) = mutation.commit() {
                            return Response::err(
                                id,
                                ResponseError::new("plan_command_failed", e.to_string()),
                            );
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        // A pending change exists (mid-turn selection); the
                        // boundary hook lands it later. Report, don't queue.
                        return Response::err(
                            id,
                            ResponseError::new("plan_command_pending", e.to_string()),
                        );
                    }
                }
                if prompt_text.is_none() {
                    return Response::ok(
                        id,
                        json!({
                            "plan_mode": ctx.plan_mode.active().unwrap_or(false),
                            "prompted": false,
                        }),
                    );
                }
            }
            let msg = prompt_text.as_deref().unwrap_or(&msg);
            // Optional session_id + run_id: when provided, we record a
            // run in the ledger so the client can correlate across
            // reconnects.
            let session_id = req
                .params
                .get("session_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            let run_id = req
                .params
                .get("run_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            let started = crate::state::now_ms();
            if !run_id.is_empty() {
                let _ = ctx.runs.start(run_id, session_id, started);
            }
            record_turn(
                &ctx.sessions,
                session_id,
                run_id,
                TurnRecord::TurnStart {
                    run_id: run_id.to_string(),
                    ts_ms: started,
                },
            );
            // G8: in orbit mode the run's AgentEnd is consumed by the event
            // pump, which was previously started lazily on the first
            // `event.subscribe`. A client that prompts without subscribing
            // would leave every run half-open forever — the exact state
            // this change exists to eliminate. Start the pump before the
            // prompt goes out so the close path is wired regardless of
            // subscriptions. Idempotent (`pump_active` guards the spawn).
            if ctx.worker.is_orbit() {
                if let Err(e) = ctx
                    .worker
                    .ensure_event_pump(&ctx.events, &ctx.sessions, &ctx.runs)
                {
                    return e;
                }
            }

            if let Err(e) = ctx.worker.ensure_started() {
                if !run_id.is_empty() {
                    let _ = ctx
                        .runs
                        .finish(run_id, crate::state::now_ms(), "spawn_failed");
                }
                record_turn(
                    &ctx.sessions,
                    session_id,
                    run_id,
                    TurnRecord::TurnEnd {
                        run_id: run_id.to_string(),
                        ts_ms: crate::state::now_ms(),
                        status: "failed".into(),
                    },
                );
                return e;
            }
            // G7-B: declare the run this turn's events belong to before the
            // prompt goes out.  The pump stamps it on every frame it pushes
            // while the slot holds it (sticky — see `set_active_run`).
            ctx.worker.set_active_run(run_id);
            // B2a resume: replay the session's persisted history into the
            // worker's live context before the prompt so a restarted daemon
            // (or a respawned engine) does not start from a blank slate.
            // Best-effort — a load or replay failure must not block the
            // prompt (the turn still runs, it just misses the history).
            // In orbit mode the engine dedupes per session id, so calling
            // this on every prompt is safe; in omp mode it is `Ok(0)`.
            if !session_id.is_empty() {
                match ctx.sessions.load_messages(session_id, RESUME_CONTEXT_LIMIT) {
                    Ok(msgs) => {
                        if let Err(e) = ctx.worker.resume_session(session_id, &msgs) {
                            eprintln!(
                                "daemon: session resume failed for {session_id} (continuing without history): {e:?}"
                            );
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "daemon: could not load session {session_id} for resume (continuing without history): {e}"
                        );
                    }
                }
            }
            let w = ctx.worker.inner.as_mut().expect("ensured");
            let resp = w.prompt(msg);
            let finished = crate::state::now_ms();
            match &resp {
                Ok(v) => {
                    if ctx.worker.is_orbit() {
                        // G8: an orbit prompt only acknowledges that the
                        // message reached the engine's run thread — the turn
                        // itself is still in flight and ends later, when the
                        // event pump forwards `AgentEnd`.  Closing the run
                        // here would make every orbit run look finished the
                        // instant it started: `in_flight_runs` pinned at 0,
                        // the session state machine's three states
                        // unreachable, and the half-open turn-log entry that
                        // `interrupted_run_closers` exists to repair could
                        // never appear in a live log.  Just ack the client;
                        // `ensure_event_pump` owns the close.
                        return Response::ok(id, v.clone());
                    }
                    // omp-compat: `prompt` blocks for the whole turn, so its
                    // return *is* the terminal state — close synchronously.
                    if !run_id.is_empty() {
                        if let Err(e) = ctx.runs.finish(run_id, finished, "ok") {
                            eprintln!("daemon: run finish failed for {run_id}: {e}");
                        }
                    }
                    record_turn(
                        &ctx.sessions,
                        session_id,
                        run_id,
                        TurnRecord::TurnEnd {
                            run_id: run_id.to_string(),
                            ts_ms: finished,
                            status: "ok".into(),
                        },
                    );
                    Response::ok(id, v.clone())
                }
                Err(e) => {
                    // The message never reached the engine (orbit: run
                    // channel closed; omp: wire error), so the run did not
                    // start at all — closing it as failed here is correct in
                    // either mode.
                    if !run_id.is_empty() {
                        if let Err(e) = ctx.runs.finish(run_id, finished, "failed") {
                            eprintln!("daemon: run finish-failed marking failed for {run_id}: {e}");
                        }
                    }
                    record_turn(
                        &ctx.sessions,
                        session_id,
                        run_id,
                        TurnRecord::TurnEnd {
                            run_id: run_id.to_string(),
                            ts_ms: finished,
                            status: "failed".into(),
                        },
                    );
                    Response::err(
                        id,
                        ResponseError::new("worker_prompt_failed", e.to_string()),
                    )
                }
            }
        }

        Command::WorkerSteer => {
            let msg = match require_str(&req.params, "message") {
                Ok(s) => s,
                Err(m) => return Response::err(id, ResponseError::new("protocol", m)),
            };
            if let Err(e) = ctx.worker.ensure_started() {
                return e;
            }
            let w = ctx.worker.inner.as_mut().expect("ensured");
            match w.steer(msg) {
                Ok(v) => Response::ok(id, v),
                Err(e) => {
                    Response::err(id, ResponseError::new("worker_steer_failed", e.to_string()))
                }
            }
        }

        Command::WorkerAbort => {
            if let Err(e) = ctx.worker.ensure_started() {
                return e;
            }
            let w = ctx.worker.inner.as_mut().expect("ensured");
            match w.abort() {
                Ok(v) => Response::ok(id, v),
                Err(e) => {
                    Response::err(id, ResponseError::new("worker_abort_failed", e.to_string()))
                }
            }
        }
        Command::WorkerReadEvent => {
            if let Err(e) = ctx.worker.ensure_started() {
                return e;
            }
            let w = ctx.worker.inner.as_mut().expect("ensured");
            match w.read_event() {
                Ok(ev) => match serde_json::to_value(ev) {
                    Ok(v) => Response::ok(id, v),
                    Err(e) => Response::err(id, ResponseError::new("internal", e.to_string())),
                },
                Err(e) => Response::err(
                    id,
                    ResponseError::new("worker_read_event_failed", e.to_string()),
                ),
            }
        }

        Command::UserAnswer => {
            let qid = match require_str(&req.params, "question_id") {
                Ok(s) => s,
                Err(m) => return Response::err(id, ResponseError::new("protocol", m)),
            };
            if qid.len() > crate::questions::MAX_QUESTION_ID_BYTES {
                return Response::err(id, ResponseError::new("protocol", "question id too long"));
            }
            let answer: crate::questions::QuestionAnswer = match req.params.get("answer") {
                Some(value) => match serde_json::from_value(value.clone()) {
                    Ok(a) => a,
                    Err(e) => {
                        return Response::err(
                            id,
                            ResponseError::new("protocol", format!("malformed answer: {e}")),
                        );
                    }
                },
                None => {
                    return Response::err(id, ResponseError::new("protocol", "answer is required"));
                }
            };
            match ctx.questions.answer(qid, answer) {
                Ok(()) => Response::ok(id, json!({ "answered": true })),
                Err(e) => Response::err(id, e.into()),
            }
        }
        Command::UserQuestionPending => {
            let pending = ctx.questions.pending();
            Response::ok(
                id,
                serde_json::to_value(pending).unwrap_or_else(|_| json!([])),
            )
        }

        // ---------------- Events (R2 3.3) ----------------
        Command::EventSubscribe => {
            let topic = match require_str(&req.params, "topic") {
                Ok(s) => s,
                Err(m) => return Response::err(id, ResponseError::new("protocol", m)),
            };
            if topic == WORKER_TOPIC
                && let Err(e) = ctx
                    .worker
                    .ensure_event_pump(&ctx.events, &ctx.sessions, &ctx.runs)
            {
                return e;
            }
            let sub_id = ctx.events.subscribe(topic, ctx.conn_id, ctx.out.clone());
            Response::ok(id, json!({ "subscription_id": sub_id, "topic": topic }))
        }

        Command::EventUnsubscribe => {
            let Some(sub_id) = req.params.get("subscription_id").and_then(Value::as_u64) else {
                return Response::err(
                    id,
                    ResponseError::new("protocol", "subscription_id (u64) is required"),
                );
            };
            let removed = ctx.events.unsubscribe(sub_id);
            Response::ok(id, json!({ "removed": removed }))
        }
    }
}

/// Record one run boundary in the session's durable turn log. Best-effort
/// and invisible to the protocol: a caller that omits `session_id` / `run_id`
/// gets no turn log. A storage failure is logged rather than propagated (the
/// ledger write next to it already ignores its own errors) — but it stays
/// visible, because the persisted log is what startup crash repair walks, so
/// a silent failure here would later surface as an unrepaired run.
/// See [`crate::server::Daemon::repair_interrupted_runs`].
fn record_turn(sessions: &SessionState, session_id: &str, run_id: &str, record: TurnRecord) {
    if session_id.is_empty() || run_id.is_empty() {
        return;
    }
    if let Err(e) = sessions.append_turn_log(session_id, &[record]) {
        eprintln!("daemon: turn log append failed for session {session_id}: {e}");
    }
}

fn session_error_response(
    id: Option<&str>,
    command: &'static str,
    e: session::SessionError,
) -> Response {
    use session::SessionError;
    let code = match &e {
        SessionError::InvalidSessionId => "invalid_session_id",
        SessionError::InvalidMessageText => "invalid_message_text",
        SessionError::InvalidListQuery => "invalid_list_query",
        SessionError::InvalidSearchQuery => "invalid_search_query",
        SessionError::InvalidLimit(_) => "invalid_limit",
        SessionError::UnknownRole(_) => "unknown_role",
        SessionError::DatabaseMissing(_) => "database_missing",
        SessionError::MalformedTurnLog { .. } => "malformed_turn_log",
        SessionError::Libsql(_) | SessionError::Io(_) | SessionError::RuntimeBuild(_) => {
            "session_io"
        }
    };
    Response::err(id, ResponseError::new(code, format!("{command}: {e}")))
}
