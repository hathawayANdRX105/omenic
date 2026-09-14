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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use serde_json::{Value, json};
use session::SessionRole;

use crate::protocol::{Command, EventFrame, Request, Response, ResponseError};
use crate::state::{EventBus, RunLedger, SessionState, require_str, require_u32};

/// Topic fed by the `rpc` worker's push event stream (R2 3.3).
pub const WORKER_TOPIC: &str = "worker";

/// Shared worker handle.  The dispatch layer takes `&mut` so concurrent
/// connections are serialized by the server's mutex.
pub struct WorkerHandle {
    inner: Option<rpc::worker::Worker>,
    omp_path: String,
    /// Whether the forwarder thread feeding [`WORKER_TOPIC`] is alive.
    /// Cleared by the forwarder itself when the worker's event channel
    /// closes (worker died or was reset), so the next subscribe respawns it.
    pump_active: Arc<AtomicBool>,
    /// orbit 模式配置（DaemonConfig 从 .oi/config.toml 的 llm 三件套解析）
    orbit_model: Option<adaptor::Model>,
}

impl WorkerHandle {
    pub fn new(omp_path: impl Into<String>, orbit_model: Option<adaptor::Model>) -> Self {
        WorkerHandle {
            inner: None,
            omp_path: omp_path.into(),
            pump_active: Arc::new(AtomicBool::new(false)),
            orbit_model,
        }
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
            let mut w = rpc::worker::Worker::new(&self.omp_path, self.orbit_model.clone())
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
        self.inner = None;
    }

    /// Start the worker-side event pump once (R2 3.3): `rpc::Worker::
    /// subscribe` hands the wire read loop to a pump thread; this daemon
    /// side forwarder drains that receiver and broadcasts [`EventFrame`]
    /// lines to every [`WORKER_TOPIC`] subscriber on the [`EventBus`].
    /// Serialized against every other worker use by the server's mutex.
    pub fn ensure_event_pump(&mut self, events: &EventBus) -> Result<(), Response> {
        if self.pump_active.load(Ordering::SeqCst) {
            return Ok(());
        }
        self.ensure_started()?;
        let w = self.inner.as_mut().expect("ensured");
        let rx = w.subscribe(WORKER_TOPIC);
        let active = Arc::clone(&self.pump_active);
        let bus = events.clone();
        active.store(true, Ordering::SeqCst);
        std::thread::spawn(move || {
            // Ends when the worker dies (rpc pump clears its subscriber
            // table).  Events with no subscribers are dropped by broadcast.
            while let Ok(event) = rx.recv() {
                let Ok(payload) = serde_json::to_value(&event) else {
                    continue;
                };
                let Ok(line) = serde_json::to_string(&EventFrame::new(WORKER_TOPIC, payload))
                else {
                    continue;
                };
                bus.broadcast(WORKER_TOPIC, &line);
            }
            active.store(false, Ordering::SeqCst);
        });
        Ok(())
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
            match ctx.sessions.ensure_session(sid, title) {
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
            let w = (*ctx.worker).inner.as_mut().expect("ensured");
            match w.ping() {
                Ok(()) => Response::ok(id, json!({ "pong": true })),
                Err(e) => {
                    Response::err(id, ResponseError::new("worker_ping_failed", e.to_string()))
                }
            }
        }

        Command::WorkerPrompt => {
            let msg = match require_str(&req.params, "message") {
                Ok(s) => s,
                Err(m) => return Response::err(id, ResponseError::new("protocol", m)),
            };
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

            if let Err(e) = ctx.worker.ensure_started() {
                if !run_id.is_empty() {
                    let _ = ctx
                        .runs
                        .finish(run_id, crate::state::now_ms(), "spawn_failed");
                }
                return e;
            }
            let w = (*ctx.worker).inner.as_mut().expect("ensured");
            let resp = w.prompt(msg);
            let finished = crate::state::now_ms();
            match &resp {
                Ok(v) => {
                    if !run_id.is_empty() {
                        let _ = ctx.runs.finish(run_id, finished, "ok");
                    }
                    Response::ok(id, v.clone())
                }
                Err(e) => {
                    if !run_id.is_empty() {
                        let _ = ctx.runs.finish(run_id, finished, "failed");
                    }
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
            let w = (*ctx.worker).inner.as_mut().expect("ensured");
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
            let w = (*ctx.worker).inner.as_mut().expect("ensured");
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
            let w = (*ctx.worker).inner.as_mut().expect("ensured");
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

        // ---------------- Events (R2 3.3) ----------------
        Command::EventSubscribe => {
            let topic = match require_str(&req.params, "topic") {
                Ok(s) => s,
                Err(m) => return Response::err(id, ResponseError::new("protocol", m)),
            };
            if topic == WORKER_TOPIC {
                if let Err(e) = ctx.worker.ensure_event_pump(&ctx.events) {
                    return e;
                }
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
