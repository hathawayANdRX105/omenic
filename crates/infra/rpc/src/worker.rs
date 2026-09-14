#![allow(dead_code)] // consumed by runner in M3
//! Worker lifecycle: spawn / steer / abort / event stream.
//!
//! A Worker wraps an `crate::client::Client` connected to `omp --mode rpc` and provides
//! a task-oriented interface for agent interaction: send a prompt, read events,
//! steer the agent, and abort when done.
//!
//! ## Lifecycle state map (#46)
//!
//! ```text
//! spawn ──► ready handshake ──► negotiate v2 ──► idle
//!           (crate::client::Client::new)  (set_auto_retry/   │
//!                                compaction off)   ├─► prompt(msg) ──┐
//!                                                  │                 ▼
//!                                                  │            event stream
//!                                                  │        (read_event / events())
//!                                                  │                 │
//!                                                  ├─► steer(msg) ◄──┘ (between events)
//!                                                  ├─► abort() ──► killed
//!                                                  └─► Drop     ──► kill process group
//! ```
//!
//! Error paths: spawn/handshake/negotiation failure → `Worker::new` errs;
//! transport death mid-call → `RpcError::ProcessExited` from prompt/steer/
//! read_event; `Drop` always kills the child even after errors.
//!
//! ## Example
//! ```ignore
//! let mut worker = worker::Worker::new("/usr/bin/omp")?;
//! worker.prompt("Write a design doc for the auth module")?;
//! for event in worker.events() {
//!     match event {
//!         WorkerEvent::Message { text } => println!("{text}"),
//!         WorkerEvent::AgentEnd => break,
//!         _ => {}
//!     }
//! }
//! worker.abort()?;
//! ```

use serde::Serialize;
use serde_json::Value;

/// Events emitted by the worker during agent execution.
///
/// R2 2.2: one dedicated variant per known omp wire event type (mirrors the
/// dsh `known-event-types` discipline); `Unknown` stays as the forward-compat
/// branch for genuinely unrecognized frames only — never as an error dump.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum WorkerEvent {
    /// Agent started processing (`agent_start`).
    AgentStart,
    /// A streamed text message (`message_start` / `message_update`).
    Message { text: String },
    /// Tool dispatch begins (`tool_execution` / `tool_execution_start`).
    ToolExecutionStart { name: String, input: Value },
    /// Tool dispatch completes (`tool_execution_end`).
    ToolExecutionEnd { name: String, result: Option<Value> },
    /// Agent finished (session idle) (`agent_end`).
    AgentEnd,
    /// Synthetic transport error: the event stream itself failed and the
    /// raw error is surfaced to the consumer instead of killing the iterator.
    Error { error: String },
    /// An unrecognized wire event (raw value for forward-compat).
    Unknown(Value),
}

/// A worker session connected to an omp agent.
///
/// Two consumption modes:
///
/// * pull (default): `prompt()` + `read_event()` / `events()`.
/// * push (R2 2.3): [`Worker::subscribe`] hands the RPC read loop to an
///   internal pump thread which forwards every event to every receiver and
///   matches command responses by id.  Command methods keep working through
///   the pump; `read_event()` / `reconnect()` become unavailable.
pub struct Worker {
    /// `None` once the pump thread owns the client.
    client: Option<crate::client::Client>,
    pump: Option<Pump>,
    /// omenic 自家引擎模式（OMENIC_WORKER_MODE=orbit）：进程内跑 orbit
    /// agent 循环（C1），事件词汇与 omp 转发层完全一致。None = omp 模式。
    orbit: Option<OrbitEngine>,
}

/// One unit of work for the pump thread.
enum Job {
    /// Send `req` to omp (the pump assigns the correlation id) and deliver
    /// the matching response frame through `reply`.
    Command {
        req: crate::client::Request,
        reply: std::sync::mpsc::Sender<Result<Value, crate::client::RpcError>>,
    },
    /// Stop the pump; dropping the client aborts and kills the omp process.
    Shutdown,
}

/// Handle on the pump thread started by the first `subscribe()`.
struct Pump {
    job_tx: std::sync::mpsc::Sender<Job>,
    subs: std::sync::Arc<std::sync::Mutex<Vec<(String, std::sync::mpsc::Sender<WorkerEvent>)>>>,
    handle: Option<std::thread::JoinHandle<()>>,
    /// PID captured at pump start (reconnect is unavailable in push mode).
    pid: u32,
}

/// How long the pump blocks in one frame read before re-checking jobs.
const PUMP_POLL: std::time::Duration = std::time::Duration::from_millis(20);

/// omenic 自家引擎（OMENIC_WORKER_MODE=orbit）：进程内跑 orbit agent
/// 循环（C1 `run_agent_streaming`），把 orbit::AgentEvent 1:1 映射成
/// [`WorkerEvent`]（词汇与 omp 转发层一致，下游零改动）。模型配置读
/// OMENIC_LLM_BASE_URL/API_KEY/MODEL/MAX_TOKENS。
struct OrbitEngine {
    model: adaptor::Model,
    ctx: adaptor::Context,
    abort_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    subs: std::sync::Arc<std::sync::Mutex<Vec<(String, std::sync::mpsc::Sender<WorkerEvent>)>>>,
    pull_push: std::sync::mpsc::Sender<WorkerEvent>,
    pull_queue: std::sync::mpsc::Receiver<WorkerEvent>,
}

impl OrbitEngine {
    fn new() -> Result<Self, crate::client::RpcError> {
        use std::sync::atomic::AtomicBool;
        let base_url = std::env::var("OMENIC_LLM_BASE_URL").map_err(|_| {
            crate::client::RpcError::Protocol(
                "OMENIC_WORKER_MODE=orbit 需要 OMENIC_LLM_BASE_URL".into(),
            )
        })?;
        let mut url = base_url.trim().trim_end_matches('/').to_string();
        if !url.ends_with("/v1") {
            url.push_str("/v1");
        }
        let api_key = std::env::var("OMENIC_LLM_API_KEY").unwrap_or_default();
        let model = std::env::var("OMENIC_LLM_MODEL").unwrap_or_else(|_| "default".into());
        let max_tokens = std::env::var("OMENIC_LLM_MAX_TOKENS")
            .ok()
            .and_then(|v| v.parse().ok());
        let (pull_push, pull_queue) = std::sync::mpsc::channel();
        Ok(OrbitEngine {
            model: adaptor::Model {
                api_key,
                model,
                base_url: Some(url),
                max_tokens,
            },
            ctx: adaptor::Context::default(),
            abort_flag: std::sync::Arc::new(AtomicBool::new(false)),
            subs: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            pull_push,
            pull_queue,
        })
    }

    fn broadcast(&self, event: WorkerEvent) {
        let mut subs = self.subs.lock().unwrap_or_else(|e| e.into_inner());
        subs.retain(|(_, tx)| tx.send(event.clone()).is_ok());
    }

    /// 同步跑一轮：阻塞调用线程（daemon 线程-per-连接），事件边产生边
    /// 广播。返回最终 assistant 文本。
    fn run_turn(&mut self, message: &str) -> String {
        use std::sync::atomic::Ordering;
        self.ctx.messages.push(adaptor::Message::user_text(message));
        self.abort_flag.store(false, Ordering::SeqCst);
        let subs = std::sync::Arc::clone(&self.subs);
        let pull_push = self.pull_push.clone();
        let backend = orbit::HttpLlm;
        let tools = tools::builtin_tools();
        let abort_flag = std::sync::Arc::clone(&self.abort_flag);
        let mut model = self.model.clone();
        let mut ctx = std::mem::take(&mut self.ctx);
        let mut reply = String::new();
        orbit::run_agent_streaming(
            &backend,
            &model,
            &mut ctx,
            &tools,
            &abort_flag,
            orbit::LoopConfig::default(),
            &mut |ev| {
                let we = match ev {
                    orbit::AgentEvent::TurnStart => WorkerEvent::AgentStart,
                    orbit::AgentEvent::AssistantText { delta } => {
                        reply.push_str(&delta);
                        WorkerEvent::Message { text: delta }
                    }
                    orbit::AgentEvent::ToolCall(spec) => WorkerEvent::ToolExecutionStart {
                        name: spec.name.clone(),
                        input: spec.args,
                    },
                    orbit::AgentEvent::ToolStart { name, .. } => WorkerEvent::Unknown(
                        serde_json::json!({ "event": "tool_start", "name": name }),
                    ),
                    orbit::AgentEvent::ToolResult { name, result, .. } => {
                        WorkerEvent::ToolExecutionEnd {
                            name,
                            result: serde_json::from_str(&result)
                                .ok()
                                .or(Some(serde_json::Value::String(result))),
                        }
                    }
                    orbit::AgentEvent::TurnEnd { .. } => WorkerEvent::AgentEnd,
                };
                {
                    let mut s = subs.lock().unwrap_or_else(|e| e.into_inner());
                    s.retain(|(_, tx)| tx.send(we.clone()).is_ok());
                }
                let _ = pull_push.send(we);
            },
        );
        drop(model);
        self.ctx = ctx;
        reply
    }
}

impl Worker {
    fn omp_worker(client: crate::client::Client) -> Self {
        Worker {
            client: Some(client),
            pump: None,
            orbit: None,
        }
    }

    /// OMENIC_WORKER_MODE=orbit 时由 [`Worker::new`] 调用。
    fn new_orbit_from_env() -> Result<Self, crate::client::RpcError> {
        Ok(Worker {
            client: None,
            pump: None,
            orbit: Some(OrbitEngine::new()?),
        })
    }
}

impl Worker {
    /// Spawn a new worker process (`omp --mode rpc`).
    ///
    /// Blocks until the initial `ready` handshake completes.
    pub fn new(omp_path: &str) -> Result<Self, crate::client::RpcError> {
        if std::env::var("OMENIC_WORKER_MODE").as_deref() == Ok("orbit") {
            return Self::new_orbit_from_env();
        }

        let client = crate::client::Client::new(omp_path)?;
        Ok(Self::omp_worker(client))
    }

    /// Spawn with a connect timeout and auto-reconnect retry count.
    pub fn new_with_opts(
        omp_path: &str,
        connect_timeout: Option<std::time::Duration>,
        max_retries: u32,
    ) -> Result<Self, crate::client::RpcError> {
        let client = crate::client::Client::new_with_opts(omp_path, connect_timeout, max_retries)?;
        Ok(Self::omp_worker(client))
    }

    /// Reconnect the underlying client (kill + respawn + renegotiate).
    ///
    /// Only available in pull mode; the pump thread owns the client once
    /// `subscribe()` has run.
    pub fn reconnect(&mut self) -> Result<(), crate::client::RpcError> {
        if self.orbit.is_some() {
            return Err(crate::client::RpcError::Protocol(
                "orbit worker has no child process to reconnect".into(),
            ));
        }
        match self.client.as_mut() {
            Some(client) => client.reconnect(),
            None => Err(crate::client::RpcError::Protocol(
                "event pump active; reconnect() unavailable".to_string(),
            )),
        }
    }

    /// Register a push subscriber (R2 2.3).
    ///
    /// The first call starts the pump thread that owns the RPC read loop.
    /// `topic` is a routing label recorded for daemon-side fan-out; every
    /// live receiver sees every event.  The receiver yields events until the
    /// worker dies or is dropped, then disconnects.
    pub fn subscribe(&mut self, topic: &str) -> std::sync::mpsc::Receiver<WorkerEvent> {
        if let Some(orbit) = self.orbit.as_mut() {
            let (tx, rx) = std::sync::mpsc::channel();
            orbit
                .subs
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((topic.to_string(), tx));
            return rx;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let Some(client) = self.client.take() else {
            let pump = self
                .pump
                .as_ref()
                .expect("pump present once the client is taken");
            pump.subs
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((topic.to_string(), tx));
            return rx;
        };
        let (job_tx, job_rx) = std::sync::mpsc::channel();
        let subs = std::sync::Arc::new(std::sync::Mutex::new(vec![(topic.to_string(), tx)]));
        let pid = client.child_pid();
        let subs_for_pump = std::sync::Arc::clone(&subs);
        let handle = std::thread::Builder::new()
            .name("omenic-rpc-pump".into())
            .spawn(move || run_pump(client, job_rx, &subs_for_pump))
            .expect("spawn event pump");
        self.pump = Some(Pump {
            job_tx,
            subs,
            handle: Some(handle),
            pid,
        });
        rx
    }

    /// Send a ping to check liveness. Returns Ok if the process responds.
    pub fn ping(&mut self) -> Result<(), crate::client::RpcError> {
        if self.orbit.is_some() {
            return Ok(());
        }
        self.command(crate::client::Request::new("ping").done())
            .map(|_| ())
    }

    /// Register tool definitions that omp may invoke through this client.
    pub fn register_external_tools(
        &mut self,
        defs: Vec<adaptor::ToolDef>,
    ) -> Result<(), crate::client::RpcError> {
        if self.orbit.is_some() {
            return Ok(());
        }
        let req = crate::client::Request::new("register_external_tools")
            .with_field("tools", defs)
            .done();
        let response = self.command(req)?;
        if response.get("success").and_then(Value::as_bool) == Some(true) {
            Ok(())
        } else {
            Err(crate::client::RpcError::Protocol(
                response
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("register_external_tools failed")
                    .to_string(),
            ))
        }
    }

    /// Send a prompt to the agent (initial task brief or follow-up).
    ///
    /// Returns the response data.  The agent will subsequently emit events;
    /// read them via `read_event()` or push-subscribe via `subscribe()`.
    pub fn prompt(&mut self, message: &str) -> Result<Value, crate::client::RpcError> {
        if let Some(orbit) = self.orbit.as_mut() {
            let reply = orbit.run_turn(message);
            return Ok(serde_json::json!({ "reply": reply, "success": true }));
        }
        let req = crate::client::Request::new("prompt")
            .with_field("message", message)
            .done();
        self.command(req)
    }

    /// Steer the running agent with an instruction.
    pub fn steer(&mut self, message: &str) -> Result<Value, crate::client::RpcError> {
        if self.orbit.is_some() {
            return Err(crate::client::RpcError::Protocol(
                "steer not supported in orbit worker mode yet".into(),
            ));
        }
        let req = crate::client::Request::new("steer")
            .with_field("message", message)
            .done();
        self.command(req)
    }

    /// Abort the current agent session.
    pub fn abort(&mut self) -> Result<Value, crate::client::RpcError> {
        if let Some(orbit) = self.orbit.as_ref() {
            use std::sync::atomic::Ordering;
            orbit.abort_flag.store(true, Ordering::SeqCst);
            return Ok(serde_json::json!({ "aborted": true }));
        }
        self.command(crate::client::Request::new("abort").done())
    }

    /// Dispatch a command: straight through the client in pull mode, or via
    /// the pump's job queue once push mode is active.
    fn command(&mut self, req: crate::client::Request) -> Result<Value, crate::client::RpcError> {
        if let Some(client) = self.client.as_mut() {
            return client.send(&req);
        }
        let pump = self
            .pump
            .as_ref()
            .expect("pump present once the client is taken");
        let (tx, rx) = std::sync::mpsc::channel();
        pump.job_tx
            .send(Job::Command { req, reply: tx })
            .map_err(|_| crate::client::RpcError::ProcessExited(None))?;
        rx.recv()
            .map_err(|_| crate::client::RpcError::ProcessExited(None))?
    }

    /// PID of the underlying omp worker process (its process group leader).
    pub fn child_pid(&self) -> u32 {
        if self.orbit.is_some() {
            return 0;
        }
        match (&self.client, &self.pump) {
            (Some(client), _) => client.child_pid(),
            (None, Some(pump)) => pump.pid,
            (None, None) => 0,
        }
    }

    /// Read the next event from the agent, blocking until one arrives.
    ///
    /// Returns `None` when the agent has no more events and the session is
    /// idle (i.e. a `prompt` or `steer` response was received without
    /// subsequent agent events).  Callers should loop until `None` and then
    /// decide whether to prompt again or abort.
    pub fn read_event(&mut self) -> Result<Option<WorkerEvent>, crate::client::RpcError> {
        if let Some(orbit) = self.orbit.as_ref() {
            return Ok(orbit.pull_queue.try_recv().ok());
        }
        let client = self.client.as_mut().ok_or_else(|| {
            crate::client::RpcError::Protocol(
                "event pump active; consume subscribe() receivers instead".to_string(),
            )
        })?;
        let raw = client.next_frame_raw()?;
        Ok(frame_to_event(raw))
    }

    /// Convenience iterator: yields events until `None` (response received).
    ///
    /// Consume with `for event in worker.events() { ... }`.
    pub fn events(&mut self) -> WorkerEvents<'_> {
        WorkerEvents { worker: self }
    }
}

/// Turn one raw omp wire frame into a [`WorkerEvent`].
/// `None` means the frame is a command `response` (consumed elsewhere).
fn frame_to_event(raw: Value) -> Option<WorkerEvent> {
    let ty = raw.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let event = match ty {
        "response" => return None,
        "agent_start" => WorkerEvent::AgentStart,
        "agent_end" => WorkerEvent::AgentEnd,
        "message_start" | "message_update" => {
            let text = raw
                .pointer("/message/content")
                .or_else(|| raw.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            WorkerEvent::Message { text }
        }
        "tool_execution" | "tool_execution_start" => {
            let name = raw
                .get("toolName")
                .or_else(|| raw.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let input = raw.get("input").cloned().unwrap_or(Value::Null);
            WorkerEvent::ToolExecutionStart { name, input }
        }
        "tool_execution_end" => {
            let name = raw
                .get("toolName")
                .or_else(|| raw.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let result = raw.get("result").cloned();
            WorkerEvent::ToolExecutionEnd { name, result }
        }
        _ => WorkerEvent::Unknown(raw),
    };
    Some(event)
}

/// Pump thread body: owns the client until shutdown or process death.
/// Commands arrive via `job_rx`; response frames are matched by id and
/// delivered to the waiting `Worker::command()` caller; every other frame
/// becomes a [`WorkerEvent`] fanned out to all live subscribers.
fn run_pump(
    mut client: crate::client::Client,
    job_rx: std::sync::mpsc::Receiver<Job>,
    subs: &std::sync::Arc<std::sync::Mutex<Vec<(String, std::sync::mpsc::Sender<WorkerEvent>)>>>,
) {
    let mut pending: std::collections::HashMap<
        String,
        std::sync::mpsc::Sender<Result<Value, crate::client::RpcError>>,
    > = std::collections::HashMap::new();
    let mut dead = false;
    while !dead {
        // Drain queued commands before blocking on a frame read.
        while let Ok(job) = job_rx.try_recv() {
            match job {
                Job::Shutdown => {
                    dead = true;
                    break;
                }
                Job::Command { mut req, reply } => {
                    let id = client.next_id_str();
                    req.id = Some(id.clone());
                    match client.send_frame(&req) {
                        Ok(()) => {
                            pending.insert(id, reply);
                        }
                        Err(e) => {
                            let _ = reply.send(Err(e));
                            dead = true;
                            break;
                        }
                    }
                }
            }
        }
        if dead {
            break;
        }
        match client.next_frame_raw_timeout(PUMP_POLL) {
            Ok(frame) => {
                let ty = frame.get("type").and_then(Value::as_str).unwrap_or("");
                if ty == "response" {
                    let id = frame.get("id").and_then(Value::as_str).unwrap_or("");
                    if let Some(reply) = pending.remove(id) {
                        let _ = reply.send(Ok(frame));
                    }
                    // Unmatched responses (e.g. abort from Client::Drop) are dropped.
                } else if let Some(event) = frame_to_event(frame) {
                    let mut guard = subs.lock().unwrap_or_else(|e| e.into_inner());
                    let mut i = 0;
                    while i < guard.len() {
                        // Receiver dropped -> unregister this subscriber.
                        if guard[i].1.send(event.clone()).is_err() {
                            guard.swap_remove(i);
                        } else {
                            i += 1;
                        }
                    }
                }
            }
            Err(crate::client::RpcError::Timeout) => {}
            Err(_) => dead = true, // process exited or transport error
        }
    }
    for (_, reply) in pending.drain() {
        let _ = reply.send(Err(crate::client::RpcError::ProcessExited(None)));
    }
    // Clearing the table closes every subscriber receiver (Disconnected),
    // then dropping the client performs the abort + process-group kill.
    subs.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(pump) = self.pump.take() {
            let _ = pump.job_tx.send(Job::Shutdown);
            if let Some(handle) = pump.handle {
                let _ = handle.join();
            }
        }
        // Pull mode: Client's Drop sends abort + kills the process.
    }
}

/// Iterator over worker events.
pub struct WorkerEvents<'a> {
    worker: &'a mut Worker,
}

impl<'a> Iterator for WorkerEvents<'a> {
    type Item = WorkerEvent;

    fn next(&mut self) -> Option<Self::Item> {
        match self.worker.read_event() {
            Ok(Some(event)) => Some(event),
            Ok(None) => None,
            Err(e) => {
                // Surface the failure as a dedicated event so the caller can
                // react without the iterator silently ending.
                Some(WorkerEvent::Error {
                    error: e.to_string(),
                })
            }
        }
    }
}
