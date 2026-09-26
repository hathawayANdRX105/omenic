#![allow(dead_code)] // consumed by runner in M3
//! Worker lifecycle: spawn / steer / abort / event stream.
//!
//! A Worker wraps an `super::client::Client` connected to `omp --mode rpc` and provides
//! a task-oriented interface for agent interaction: send a prompt, read events,
//! steer the agent, and abort when done.
//!
//! ## Lifecycle state map (#46)
//!
//! ```text
//! spawn ──► ready handshake ──► negotiate v2 ──► idle
//!           (super::client::Client::new)  (set_auto_retry/   │
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
//!         WorkerEvent::AgentEnd { .. } => break,
//!         _ => {}
//!     }
//! }
//! worker.abort()?;
//! ```

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Map orbit's loop-level stop reason onto the snake_case wire string
/// [`WorkerEvent::AgentEnd`] carries — `end_turn` / `max_tokens` / `aborted`
/// / `error` / `max_turns` — so a clean turn end, an abort, an error and the
/// turn cap stay distinguishable downstream.
/// Build the user message the model sees: plain text when there are no
/// attachments, otherwise a block list carrying the text first and the
/// images after (OpenAI's multimodal ordering).
fn user_message(text: &str, attachments: &[session::Attachment]) -> llm::Message {
    if attachments.is_empty() {
        return llm::Message::user_text(text);
    }
    let mut blocks = vec![llm::Block::Text { text: text.into() }];
    blocks.extend(attachments.iter().map(|a| llm::Block::Image {
        media_type: a.media_type.clone(),
        data: a.data.clone(),
    }));
    llm::Message {
        role: llm::Role::User,
        content: llm::Content::Blocks(blocks),
    }
}

fn turn_stop_to_string(s: &protocol::events::TurnStop) -> String {
    match s {
        protocol::events::TurnStop::EndTurn => "end_turn",
        protocol::events::TurnStop::MaxTokens => "max_tokens",
        protocol::events::TurnStop::Aborted => "aborted",
        protocol::events::TurnStop::Error => "error",
        protocol::events::TurnStop::MaxTurns => "max_turns",
    }
    .to_string()
}

/// Events emitted by the worker during agent execution.
///
/// R2 2.2: one dedicated variant per known omp wire event type (mirrors the
/// dsh `known-event-types` discipline); `Unknown` stays as the forward-compat
/// branch for genuinely unrecognized frames only — never as an error dump.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum WorkerEvent {
    /// Agent started processing (`agent_start`).
    AgentStart,
    /// A streamed text message (`message_start` / `message_update`).
    Message { text: String },
    /// A streamed chain-of-thought delta (`reasoning`). Display-only; never
    /// replayed into the context as a user/assistant message.
    Reasoning { delta: String },
    /// Tool dispatch begins (`tool_execution` / `tool_execution_start`).
    ToolExecutionStart { name: String, input: Value },
    /// Tool dispatch completes (`tool_execution_end`).
    ToolExecutionEnd { name: String, result: Option<Value> },
    /// Agent finished (session idle) (`agent_end`).
    ///
    /// `stop_reason` mirrors orbit's `TurnStop`: how the turn ended (clean,
    /// aborted, errored, or a budget cap).  Defaults to empty for legacy
    /// wire frames and omp-compat peers that never send it — downstream
    /// bookkeeping treats empty as "unknown, not necessarily clean".
    AgentEnd {
        #[serde(default)]
        stop_reason: String,
    },
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
    client: Option<super::client::Client>,
    pump: Option<Pump>,
    /// omenic 自家引擎模式（OMENIC_WORKER_MODE=orbit）：进程内跑 orbit
    /// agent 循环（C1），事件词汇与 omp 转发层完全一致。None = omp 模式。
    orbit: Option<OrbitEngine>,
    /// Last `abort` was a user request (ESC), not a kill/teardown. The
    /// event pump reads this when closing the run: a user-initiated stop
    /// books the ledger as `"paused"` (resumable) instead of `"aborted"`
    /// (killed). Only consulted on `stop_reason == "aborted"` frames, so a
    /// stale value from an earlier prompt is harmless.
    pub user_abort: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// One unit of work for the pump thread.
enum Job {
    /// Send `req` to omp (the pump assigns the correlation id) and deliver
    /// the matching response frame through `reply`.
    Command {
        req: super::client::Request,
        reply: std::sync::mpsc::Sender<Result<Value, super::client::RpcError>>,
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

/// Explicit-completion guard: when the model stops without `mark_done`, the
/// loop is forced to continue up to this many times per prompt before an
/// unmarked stop is allowed. ponytail: grok's `max_fires_per_prompt=2`
/// default; promote to config if a per-workspace tuning shows up.
const MARK_GUARD_MAX_FIRES: u32 = 2;

/// Queue of passive notifications for the running loop (background job
/// completions and friends). Shared between the jobs registry's
/// `on_job_done` hook and the lazily-spawned engine.
pub type AsideQueue = std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<llm::Message>>>;

/// orbit-engine configuration resolved by the host (the daemon) from the
/// assembled plugin container: the session cwd for `AGENTS.md` discovery,
/// the turn cap, the compaction policy, and the tool catalog. Nothing here
/// is read from ambient process state — every knob arrives explicitly, the
/// same contract orbit's `LoopConfig` keeps with the loop.
#[derive(Clone)]
pub struct OrbitConfig {
    /// `AGENTS.md` discovery root; `None` skips the instruction lookup
    /// entirely (orbit `LoopConfig::instruction_cwd`).
    pub cwd: Option<std::sync::Arc<Path>>,
    /// Cap on LLM round-trips per run (orbit `LoopConfig::max_turns`).
    pub max_turns: usize,
    /// Compaction policy supplying the maintenance hook's budget and
    /// verbatim-window region (`harness.compaction`). The summary stream
    /// itself goes through the loop's own backend.
    pub compaction: std::sync::Arc<agent_loop::compaction::CharBudgetPolicy>,
    /// Tool catalog (`harness.tools`), adapted onto orbit's tool trait at
    /// the seam — see [`orbit_tools`].
    pub catalog: std::sync::Arc<tools::ToolCatalog>,
    /// External MCP tools brought up once at daemon start, shared by every
    /// engine (re)spawn. `Arc<Vec<Arc<_>>>` because `OrbitSetup` is cloned
    /// per worker respawn and a `Box<dyn Tool>` cannot be shared; the
    /// `Tool` trait's `Send + Sync` supertraits make the Arc itself
    /// `Send + Sync`, which the engine's run thread requires. Merged into
    /// the per-engine tool list by [`combined_tools`]. Empty (no servers
    /// configured, or every server skipped) leaves engine behavior
    /// identical to the pre-MCP engine.
    pub mcp_tools: std::sync::Arc<Vec<std::sync::Arc<dyn tools::Tool>>>,
    /// Job and terminal tools (`omenic-harness-tools::jobs_terminal`), shared
    /// like `mcp_tools` and for the same reason: the registries they act on
    /// must outlive any single engine. A job started in one turn has to still
    /// be listable in the next, and a terminal session has to survive the tool
    /// call that opened it — both are properties of the daemon, not of an
    /// engine instance. Empty leaves the tool list exactly as it was before
    /// this family existed.
    pub session_tools: std::sync::Arc<Vec<std::sync::Arc<dyn tools::Tool>>>,
    /// Per-turn `plan:policy` section provider (plan mode). The engine
    /// calls it before every prompt: a non-empty result replaces the
    /// default system prompt with `base + section`, so flipping plan mode
    /// mid-session lands on the next turn. `None` = always default build
    /// (pre-plan-mode behavior).
    pub plan_policy_section: Option<std::sync::Arc<dyn Fn() -> String + Send + Sync>>,
    /// Asides queued for the loop: finished background jobs, etc.
    /// Shared with the jobs registry's `on_job_done` hook so a completion
    /// lands here even though the engine is spawned lazily.
    pub aside_queue: AsideQueue,
}

/// orbit-mode construction bundle: the model, the streaming backend, and the
/// container-resolved [`OrbitConfig`]. The backend is injectable so a test
/// can substitute a scripted backend; production passes `agent_loop::orbit::HttpLlm`.
/// Cloned by the daemon on each (re)spawn of the worker — `reset()` drops
/// the engine and the next prompt rebuilds it from the same setup.
#[derive(Clone)]
pub struct OrbitSetup {
    pub model: llm::Model,
    pub backend: std::sync::Arc<dyn agent_loop::orbit::LlmBackend + Send + Sync>,
    pub config: OrbitConfig,
}

/// harness `Tool` -> omenic `tools::Tool`. orbit's loop dispatches
/// `&[Box<dyn tools::Tool>]`; the container's catalog speaks the harness
/// trait. The two are deliberately not unified (C6) — this shim is the whole
/// bridge, built once per engine.
struct HarnessTool {
    name: String,
    description: String,
    parameters: Value,
    inner: std::sync::Arc<dyn protocol::Tool>,
    /// The engine's shared abort flag — the same `AtomicBool` the orbit loop
    /// polls, so an abort reaches the harness tool mid-flight.
    abort: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl HarnessTool {
    fn new(
        tool: std::sync::Arc<dyn protocol::Tool>,
        abort: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        let spec = tool.spec();
        HarnessTool {
            name: spec.name,
            description: spec.description,
            parameters: spec.params_schema,
            inner: tool,
            abort,
        }
    }
}

impl tools::Tool for HarnessTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> String {
        self.description.clone()
    }
    fn parameters(&self) -> Value {
        self.parameters.clone()
    }
    fn execute(
        &self,
        args: &Value,
        _signal: &std::sync::atomic::AtomicBool,
    ) -> Result<String, tools::ToolError> {
        let abort = protocol::AbortSignal::from_flag(std::sync::Arc::clone(&self.abort));
        match self.inner.execute(args, &abort) {
            Ok(result) => Ok(result.output),
            Err(protocol::ToolError::Aborted) => {
                Err(tools::ToolError::Message("tool aborted".into()))
            }
            Err(e) => Ok(format!("error: {e}")),
        }
    }
}

/// Adapt the container's catalog onto orbit's `&[Box<dyn tools::Tool>]` shape.
fn orbit_tools(
    catalog: &tools::ToolCatalog,
    abort: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Vec<Box<dyn tools::Tool>> {
    catalog
        .all()
        .into_iter()
        .map(|t| {
            Box::new(HarnessTool::new(t, std::sync::Arc::clone(&abort))) as Box<dyn tools::Tool>
        })
        .collect()
}

/// Shared MCP tool -> per-engine `Box<dyn tools::Tool>`. The daemon owns the
/// live MCP connections behind `Arc<dyn tools::Tool>` handles (brought up
/// once at start, shared across respawns); each engine spawn wraps every
/// shared handle in this thin delegating shim so the loop keeps its
/// `&[Box<dyn tools::Tool>]` shape — mirroring how [`HarnessTool`] wraps the
/// catalog's `Arc<dyn harness Tool>`. `execute` forwards the caller's signal
/// verbatim: the orbit loop hands it the engine's abort flag, so an abort
/// reaches an in-flight MCP round trip (the transport polls that flag).
struct McpToolShim {
    name: String,
    description: String,
    parameters: Value,
    inner: std::sync::Arc<dyn tools::Tool>,
}

impl McpToolShim {
    fn new(inner: std::sync::Arc<dyn tools::Tool>) -> Self {
        McpToolShim {
            name: inner.name().to_string(),
            description: inner.description(),
            parameters: inner.parameters(),
            inner,
        }
    }
}

impl tools::Tool for McpToolShim {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> String {
        self.description.clone()
    }
    fn parameters(&self) -> Value {
        self.parameters.clone()
    }
    fn execute(
        &self,
        args: &Value,
        signal: &std::sync::atomic::AtomicBool,
    ) -> Result<String, tools::ToolError> {
        self.inner.execute(args, signal)
    }
}

/// The engine's full tool list: catalog tools first (registration order),
/// then one [`McpToolShim`] per shared MCP tool, then one per shared
/// job/terminal tool. Split out from [`OrbitEngine::new`] so the merge contract
/// is unit-testable without a daemon; with both shared slices empty it is
/// exactly the pre-existing [`orbit_tools`] result.
///
/// A shared tool is never deduplicated against the catalog: the two families
/// come from different sources and a name collision would mean the catalog
/// silently shadowing a shared tool (or vice versa) rather than a deliberate
/// override. Both are shimmed, so a collision shows up as two entries and is
/// caught by whichever layer validates uniqueness.
pub fn combined_tools(
    catalog: &tools::ToolCatalog,
    mcp_tools: &[std::sync::Arc<dyn tools::Tool>],
    session_tools: &[std::sync::Arc<dyn tools::Tool>],
    abort: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Vec<Box<dyn tools::Tool>> {
    let mut tools = orbit_tools(catalog, std::sync::Arc::clone(&abort));
    tools.extend(
        mcp_tools
            .iter()
            .chain(session_tools.iter())
            .map(|t| Box::new(McpToolShim::new(std::sync::Arc::clone(t))) as Box<dyn tools::Tool>),
    );
    tools
}

/// omenic 自家引擎（OMENIC_WORKER_MODE=orbit）：进程内跑 orbit agent
/// 循环（C1 `run_agent_streaming`），把 protocol::events::AgentEvent 1:1 映射成
/// [`WorkerEvent`]（词汇与 omp 转发层一致，下游零改动）。模型配置由
/// DaemonConfig 从 `.oi/config.toml` 的 llm 三件套解析后传入；cwd /
/// max_turns / 压缩策略 / 工具集由宿主从装配容器解析后经
/// [`OrbitSetup`] 传入。
struct OrbitEngine {
    model: llm::Model,
    backend: std::sync::Arc<dyn agent_loop::orbit::LlmBackend + Send + Sync>,
    ctx: std::sync::Arc<std::sync::Mutex<llm::Context>>,
    /// Session id already replayed into [`Self::ctx`]. The engine's context
    /// is rebuilt from scratch on every (re)spawn, so a session may only be
    /// resumed once per engine: a second call with the same id is a no-op
    /// that returns the count recorded on the first, and a *different* id
    /// replaces the replay (e.g. the daemon switched sessions on the same
    /// live worker).
    resumed_session: Option<String>,
    abort_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    subs: std::sync::Arc<std::sync::Mutex<Vec<(String, std::sync::mpsc::Sender<WorkerEvent>)>>>,
    pull_push: std::sync::mpsc::Sender<WorkerEvent>,
    pull_queue: std::sync::mpsc::Receiver<WorkerEvent>,
    /// prompt → 专用 run 线程：LLM 调用不占 dispatch 锁，abort 随时可达
    run_tx: std::sync::mpsc::Sender<llm::Message>,
    /// `AGENTS.md` 发现根（orbit `LoopConfig::instruction_cwd`）。
    cwd: Option<std::sync::Arc<Path>>,
    /// 每轮 run 的 LLM 往返上限（orbit `LoopConfig::max_turns`）。
    max_turns: usize,
    /// 压缩策略（`harness.compaction`），由 maintain 钩子消费。
    compaction: std::sync::Arc<agent_loop::compaction::CharBudgetPolicy>,
    /// plan:policy 段提供者（plan mode）；每轮 prompt 前调用。
    plan_policy_section: Option<std::sync::Arc<dyn Fn() -> String + Send + Sync>>,
    /// Steering queue for inter-turn user instructions. Drained by the
    /// loop's `get_steering` pull at the top of every round.
    steering_queue: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<llm::Message>>>,
    /// Passive notifications (background job completions). Drained by the
    /// loop's `get_aside` pull at the same boundary, but never extending
    /// the run.
    aside_queue: AsideQueue,
}

impl OrbitEngine {
    fn new(setup: OrbitSetup) -> Self {
        use std::sync::atomic::AtomicBool;
        let OrbitSetup {
            model,
            backend,
            config:
                OrbitConfig {
                    cwd,
                    max_turns,
                    compaction,
                    catalog,
                    mcp_tools,
                    session_tools,
                    plan_policy_section,
                    aside_queue,
                },
        } = setup;
        let (pull_push, pull_queue) = std::sync::mpsc::channel();
        let (run_tx, run_rx) = std::sync::mpsc::channel::<llm::Message>();
        let abort_flag = std::sync::Arc::new(AtomicBool::new(false));
        // Catalog tools + shared MCP tools (each shimmed per spawn); abort
        // flag shared so an engine abort reaches both tool families.
        let tools = combined_tools(
            &catalog,
            &mcp_tools,
            &session_tools,
            std::sync::Arc::clone(&abort_flag),
        );
        let engine = OrbitEngine {
            model: model.clone(),
            backend: Arc::clone(&backend),
            ctx: std::sync::Arc::new(std::sync::Mutex::new(llm::Context::default())),
            resumed_session: None,
            abort_flag: std::sync::Arc::clone(&abort_flag),
            subs: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            pull_push,
            pull_queue,
            run_tx,
            cwd,
            max_turns,
            compaction,
            plan_policy_section,
            steering_queue: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::VecDeque::new(),
            )),
            aside_queue,
        };
        // 专用 run 线程：串行消费 prompt，LLM 调用不占 dispatch 锁，
        // abort 标志（Arc）随时可从别的连接置位
        let run_model = engine.model.clone();
        let run_backend = Arc::clone(&engine.backend);
        let run_ctx = std::sync::Arc::clone(&engine.ctx);
        let run_abort = std::sync::Arc::clone(&engine.abort_flag);
        let run_subs = std::sync::Arc::clone(&engine.subs);
        let run_pull = engine.pull_push.clone();
        let run_cwd = engine.cwd.clone();
        let run_max_turns = engine.max_turns;
        let run_compaction = std::sync::Arc::clone(&engine.compaction);
        let run_plan_section = engine.plan_policy_section.clone();
        let run_steering = std::sync::Arc::clone(&engine.steering_queue);
        let run_aside = std::sync::Arc::clone(&engine.aside_queue);
        std::thread::Builder::new()
            .name("omenic-orbit-worker".into())
            .spawn(move || {
                while let Ok(message) = run_rx.recv() {
                    run_abort.store(false, std::sync::atomic::Ordering::SeqCst);
                    // panic 恢复：单轮 turn 失败不杀死整条管线，事件流里
                    // 出现 agent_end 让消费端复位
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let mut ctx = run_ctx.lock().unwrap_or_else(|e| e.into_inner());
                        ctx.messages.push(message);
                        // plan mode: recompute the system prompt per turn so a
                        // `/plan` flip lands on the next prompt. Empty section
                        // (plan mode off, or no provider) leaves `None` and
                        // orbit builds its own default.
                        ctx.system_prompt = run_plan_section.as_ref().and_then(|provider| {
                            let section = provider();
                            if section.is_empty() {
                                None
                            } else {
                                Some(format!(
                                    "{}\n\n{section}",
                                    agent_loop::orbit::build_system_prompt(run_cwd.as_deref())
                                ))
                            }
                        });
                        // 压缩钩子：容器解析出的 policy 供预算/最近窗口，
                        // 摘要流走 loop 自己的后端（orbit 接缝，G6 缺口 A）。
                        let compaction = std::sync::Arc::clone(&run_compaction);
                        let maintain =
                            |b: &dyn agent_loop::orbit::LlmBackend,
                             m: &llm::Model,
                             c: &mut llm::Context,
                             s: &std::sync::atomic::AtomicBool| {
                                agent_loop::orbit::compact_context_with(
                                    b,
                                    m,
                                    c,
                                    s,
                                    None,
                                    &compaction,
                                );
                            };
                        // Explicit-completion guard (freebuff task_completed
                        // shape): the run may end only after the model calls
                        // `mark_done`. Per-prompt state, created in the loop
                        // body so each prompt starts unmarked.
                        let marked = std::cell::Cell::new(false);
                        let fires = std::cell::Cell::new(0u32);
                        agent_loop::orbit::run_agent_streaming(
                            run_backend.as_ref(),
                            &run_model,
                            &mut ctx,
                            &tools,
                            &run_abort,
                            agent_loop::orbit::LoopConfig {
                                context_log: None,
                                max_turns: run_max_turns,
                                maintain: Some(&maintain),
                                get_steering: Some(&|| {
                                    let mut q =
                                        run_steering.lock().unwrap_or_else(|e| e.into_inner());
                                    q.drain(..).collect::<Vec<_>>()
                                }),
                                get_aside: Some(&|| {
                                    let mut q = run_aside.lock().unwrap_or_else(|e| e.into_inner());
                                    q.drain(..).collect::<Vec<_>>()
                                }),
                                get_follow_up: None,
                                should_continue: Some(&|| {
                                    if marked.get() {
                                        return false;
                                    }
                                    let n = fires.get();
                                    if n < MARK_GUARD_MAX_FIRES {
                                        fires.set(n + 1);
                                        true
                                    } else {
                                        false
                                    }
                                }),
                                instruction_cwd: run_cwd.as_deref(),
                            },
                            &mut |ev| {
                                let we = match ev {
                                    protocol::events::AgentEvent::TurnStart => {
                                        WorkerEvent::AgentStart
                                    }
                                    protocol::events::AgentEvent::AssistantText { delta } => {
                                        WorkerEvent::Message { text: delta }
                                    }
                                    protocol::events::AgentEvent::AssistantReasoning { delta } => {
                                        WorkerEvent::Reasoning { delta }
                                    }
                                    protocol::events::AgentEvent::ToolCall(spec) => {
                                        if spec.name == "mark_done" {
                                            marked.set(true);
                                        }
                                        WorkerEvent::ToolExecutionStart {
                                            name: spec.name,
                                            input: spec.args,
                                        }
                                    }
                                    protocol::events::AgentEvent::ToolStart { name, .. } => {
                                        WorkerEvent::Unknown(serde_json::json!(
                                            { "name": name }
                                        ))
                                    }
                                    protocol::events::AgentEvent::ToolResult {
                                        name,
                                        result,
                                        ..
                                    } => WorkerEvent::ToolExecutionEnd {
                                        name,
                                        result: serde_json::from_str(&result)
                                            .ok()
                                            .or(Some(serde_json::Value::String(result))),
                                    },
                                    protocol::events::AgentEvent::TurnEnd { stop_reason } => {
                                        WorkerEvent::AgentEnd {
                                            stop_reason: turn_stop_to_string(&stop_reason),
                                        }
                                    }
                                };
                                let mut s = run_subs.lock().unwrap_or_else(|e| e.into_inner());
                                s.retain(|(_, tx)| tx.send(we.clone()).is_ok());
                                let _ = run_pull.send(we);
                            },
                        );
                    }));
                    if result.is_err() {
                        eprintln!("[orbit-worker] turn panicked, recovered");
                        let ev = WorkerEvent::AgentEnd {
                            stop_reason: "error".to_string(),
                        };
                        let mut s = run_subs.lock().unwrap_or_else(|e| e.into_inner());
                        s.retain(|(_, tx)| tx.send(ev.clone()).is_ok());
                        let _ = run_pull.send(ev);
                    }
                }
            })
            .expect("spawn orbit worker thread");
        engine
    }

    fn broadcast(&self, event: WorkerEvent) {
        let mut subs = self.subs.lock().unwrap_or_else(|e| e.into_inner());
        subs.retain(|(_, tx)| tx.send(event.clone()).is_ok());
    }

    /// Replay persisted session messages into the engine's context before a
    /// prompt (B2a resume). Only user/assistant text is carried: orbit
    /// context is a user/assistant text loop, so `System` and `Tool` rows
    /// are skipped.
    ///
    /// Idempotent per session, and isolating across sessions — the engine's
    /// context is rebuilt on every (re)spawn, and the daemon calls this on
    /// *every* prompt that carries a `session_id`:
    /// - same session as the last resume: append nothing (the history is
    ///   already in the context), report the current context length.
    /// - a *different* session: clear the context (drop the previous
    ///   session's history — it would otherwise linger and cross-contaminate
    ///   this session's LLM requests), then append this session's rows.
    ///
    /// Returns the context length *after* the call (`0` for an empty
    /// `session_id`).
    fn resume_orbit_context(
        &mut self,
        session_id: &str,
        messages: &[session::SessionMessage],
    ) -> usize {
        if session_id.is_empty() {
            return 0;
        }
        let mut ctx = self.ctx.lock().unwrap_or_else(|e| e.into_inner());
        if self.resumed_session.as_deref() != Some(session_id) {
            // New or switched session: drop any previous session's history so
            // the context only ever carries one session's context at a time,
            // then append this session's rows.
            ctx.messages.clear();
            for m in messages.iter() {
                match m.role {
                    session::SessionRole::User => {
                        ctx.messages.push(user_message(&m.text, &m.attachments));
                    }
                    session::SessionRole::Assistant => {
                        ctx.messages
                            .push(llm::Message::assistant_text(m.text.clone()));
                    }
                    session::SessionRole::System | session::SessionRole::Tool => {}
                }
            }
            self.resumed_session = Some(session_id.to_string());
        }
        ctx.messages.len()
    }
}

impl Worker {
    fn omp_worker(client: super::client::Client) -> Self {
        Worker {
            client: Some(client),
            pump: None,
            orbit: None,
            user_abort: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// omp 兼容构造（`orbit` 为 None）或 omenic 自家引擎（Some：模型 +
    /// 后端 + 容器解析出的 [`OrbitConfig`]）。
    pub fn new(omp_path: &str, orbit: Option<OrbitSetup>) -> Result<Self, super::client::RpcError> {
        if let Some(setup) = orbit {
            return Ok(Worker {
                client: None,
                pump: None,
                orbit: Some(OrbitEngine::new(setup)),
                user_abort: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            });
        }
        let client = super::client::Client::new(omp_path)?;
        Ok(Self::omp_worker(client))
    }

    /// Spawn with a connect timeout and auto-reconnect retry count.
    pub fn new_with_opts(
        omp_path: &str,
        connect_timeout: Option<std::time::Duration>,
        max_retries: u32,
    ) -> Result<Self, super::client::RpcError> {
        let client = super::client::Client::new_with_opts(omp_path, connect_timeout, max_retries)?;
        Ok(Self::omp_worker(client))
    }

    /// Reconnect the underlying client (kill + respawn + renegotiate).
    ///
    /// Only available in pull mode; the pump thread owns the client once
    /// `subscribe()` has run.
    pub fn reconnect(&mut self) -> Result<(), super::client::RpcError> {
        if self.orbit.is_some() {
            return Err(super::client::RpcError::Protocol(
                "orbit worker has no child process to reconnect".into(),
            ));
        }
        match self.client.as_mut() {
            Some(client) => client.reconnect(),
            None => Err(super::client::RpcError::Protocol(
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
    pub fn ping(&mut self) -> Result<(), super::client::RpcError> {
        if self.orbit.is_some() {
            return Ok(());
        }
        self.command(super::client::Request::new("ping").done())
            .map(|_| ())
    }

    /// Register tool definitions that omp may invoke through this client.
    pub fn register_external_tools(
        &mut self,
        defs: Vec<llm::ToolDef>,
    ) -> Result<(), super::client::RpcError> {
        if self.orbit.is_some() {
            return Ok(());
        }
        let req = super::client::Request::new("register_external_tools")
            .with_field("tools", defs)
            .done();
        let response = self.command(req)?;
        if response.get("success").and_then(Value::as_bool) == Some(true) {
            Ok(())
        } else {
            Err(super::client::RpcError::Protocol(
                response
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("register_external_tools failed")
                    .to_string(),
            ))
        }
    }

    /// Rebuild a resume of session history before a prompt (B2a).
    ///
    /// In orbit mode the engine's context was rebuilt from scratch on (re)
    /// spawn — a daemon restart, a worker `reset()` — so the persisted
    /// session's recent messages are replayed into it here, ahead of
    /// [`Self::prompt`], and the turn does not start from a blank slate.
    /// `System` / `Tool` rows are dropped: orbit context is a user/assistant
    /// text loop. The engine dedupes per session id: a repeated call for the
    /// same session (the daemon calls this on every prompt that carries one)
    /// appends nothing new; a call for a *different* session clears the
    /// previous session's history first so contexts never cross-contaminate.
    /// The returned `usize` is the engine context's message count *after*
    /// the call (`0` in omp mode).
    ///
    /// In omp mode (no engine) there is nothing to resume: `Ok(0)`.
    pub fn resume_session(
        &mut self,
        session_id: &str,
        messages: &[session::SessionMessage],
    ) -> Result<usize, super::client::RpcError> {
        let Some(orbit) = self.orbit.as_mut() else {
            return Ok(0);
        };
        Ok(orbit.resume_orbit_context(session_id, messages))
    }

    /// Send a prompt to the agent (initial task brief or follow-up).
    ///
    /// Returns the response data.  The agent will subsequently emit events;
    /// read them via `read_event()` or push-subscribe via `subscribe()`.
    pub fn prompt(
        &mut self,
        message: &str,
        attachments: &[session::Attachment],
    ) -> Result<Value, super::client::RpcError> {
        if let Some(orbit) = self.orbit.as_mut() {
            // 事件经订阅管线推送；本调用立即返回 ack
            orbit
                .run_tx
                .send(user_message(message, attachments))
                .map_err(|e| super::client::RpcError::Protocol(e.to_string()))?;
            return Ok(serde_json::json!({ "started": true }));
        }
        // omp mode: the external worker owns its own wire format and takes a
        // plain string, so images do not ride along there.
        let req = super::client::Request::new("prompt")
            .with_field("message", message)
            .done();
        self.command(req)
    }

    /// Steer the running agent with an instruction.
    pub fn steer(&mut self, message: &str) -> Result<Value, super::client::RpcError> {
        if let Some(orbit) = self.orbit.as_ref() {
            // Push to orbit steering queue as a user message; the loop's
            // get_steering pull drains it into the context next round.
            orbit
                .steering_queue
                .lock()
                .unwrap()
                .push_back(llm::Message::user_text(message));
            return Ok(serde_json::json!({ "steered": true }));
        }
        let req = super::client::Request::new("steer")
            .with_field("message", message)
            .done();
        self.command(req)
    }

    /// Abort the current agent session.
    pub fn abort(&mut self) -> Result<Value, super::client::RpcError> {
        use std::sync::atomic::Ordering;
        // A user-initiated abort books the run as paused, not killed.
        self.user_abort.store(true, Ordering::SeqCst);
        if let Some(orbit) = self.orbit.as_ref() {
            orbit.abort_flag.store(true, Ordering::SeqCst);
            return Ok(serde_json::json!({ "aborted": true }));
        }
        self.command(super::client::Request::new("abort").done())
    }

    /// Dispatch a command: straight through the client in pull mode, or via
    /// the pump's job queue once push mode is active.
    fn command(&mut self, req: super::client::Request) -> Result<Value, super::client::RpcError> {
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
            .map_err(|_| super::client::RpcError::ProcessExited(None))?;
        rx.recv()
            .map_err(|_| super::client::RpcError::ProcessExited(None))?
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
    pub fn read_event(&mut self) -> Result<Option<WorkerEvent>, super::client::RpcError> {
        if let Some(orbit) = self.orbit.as_ref() {
            return Ok(orbit.pull_queue.try_recv().ok());
        }
        let client = self.client.as_mut().ok_or_else(|| {
            super::client::RpcError::Protocol(
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
        "agent_end" => WorkerEvent::AgentEnd {
            stop_reason: raw
                .get("stop_reason")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        },
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
    mut client: super::client::Client,
    job_rx: std::sync::mpsc::Receiver<Job>,
    subs: &std::sync::Arc<std::sync::Mutex<Vec<(String, std::sync::mpsc::Sender<WorkerEvent>)>>>,
) {
    let mut pending: std::collections::HashMap<
        String,
        std::sync::mpsc::Sender<Result<Value, super::client::RpcError>>,
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
            Err(super::client::RpcError::Timeout) => {}
            Err(_) => dead = true, // process exited or transport error
        }
    }
    for (_, reply) in pending.drain() {
        let _ = reply.send(Err(super::client::RpcError::ProcessExited(None)));
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
