//! Agent loop: stream → tool_calls → execute → backfill → repeat.
//!
//! Port of pi-from-scratch `src/agent.ts`, aligned with oh-my-pi's loop
//! semantics (pure loop + host config). The invariants:
//! 1. Every tool_call gets a matching tool_result (API hard constraint).
//! 2. max_tokens truncation → tools NOT executed; error results backfilled
//!    so the model resends complete args.
//! 3. Abort → pending tool_calls dropped and the assistant message recorded
//!    without them, so a restored session never trips the API.
//! 4. Compaction failure → original context kept untouched.
//! 5. The loop never trusts the model to stop: `LoopConfig::max_turns`
//!    bounds LLM round-trips and compaction is a host-provided policy.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use llm::{Context, Message, Model, StopReason, StreamEvent, ToolDef};
pub use protocol::events::{AgentEvent, TurnStop};
use tools::{Tool, def};

// The compaction policy (char-budget trigger ~4 chars/token ≈ 30k tokens,
// verbatim recent window) lives in `omenic-harness-compaction` (C4); the
// public knobs stay re-exported on the orbit path.
// ponytail: fixed budget — `Model` carries no context-window field; derive
// it from provider metadata once one exists.
pub use crate::compaction::{KEEP_RECENT_CHARS, KEEP_RECENT_MIN};

/// LLM backend abstraction: the only seam between loop and network,
/// so invariants are testable offline with scripted streams.
pub trait LlmBackend {
    /// Live streaming: invoke `emit` for each event as it arrives.
    fn stream_cb(
        &self,
        model: &Model,
        context: &Context,
        tools: &[ToolDef],
        signal: &AtomicBool,
        emit: &mut dyn FnMut(&StreamEvent),
    );

    /// Collecting wrapper: default impl gathers all events into a Vec.
    fn stream(
        &self,
        model: &Model,
        context: &Context,
        tools: &[ToolDef],
        signal: &AtomicBool,
    ) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        self.stream_cb(model, context, tools, signal, &mut |ev| {
            events.push(ev.clone());
        });
        events
    }
}

/// Production backend: real OpenAI-compatible HTTP streaming.
pub struct HttpLlm;

impl LlmBackend for HttpLlm {
    fn stream_cb(
        &self,
        model: &Model,
        context: &Context,
        tools: &[ToolDef],
        signal: &AtomicBool,
        emit: &mut dyn FnMut(&StreamEvent),
    ) {
        llm::stream_cb(model, context, tools, signal, emit)
    }
}

/// Host context-maintenance hook (compaction etc.), run before each model call.
pub type MaintainFn<'a> = &'a dyn Fn(&dyn LlmBackend, &Model, &mut Context, &AtomicBool);

pub struct LoopConfig<'a> {
    /// Evidence log; assistant/tool_result/summary messages are appended
    /// as the loop produces them.
    pub context_log: Option<&'a ContextLog>,
    /// Hard cap on LLM round-trips per run. When exceeded the loop emits
    /// `TurnEnd { MaxTurns }` and returns instead of trusting the model to
    /// stop. Default [`DEFAULT_MAX_TURNS`].
    pub max_turns: usize,
    /// Context maintenance run before each model call. Compaction lives
    /// here as a host policy (omp `SessionMaintenance`), not inside the
    /// loop; pass `None` for hosts that manage context themselves.
    pub maintain: Option<MaintainFn<'a>>,
    /// Pull queued steering messages (user interjections). Drained at the
    /// top of every LLM round and appended to the context as-is, so the
    /// model sees them before its next call. Remaining messages stay in
    /// the host queue when the loop stops.
    pub get_steering: Option<&'a dyn Fn() -> Vec<Message>>,
    /// Pull passive notifications (a finished background job, a late
    /// diagnostic) drained at the same step boundary as steering, but
    /// *without* extending the run: an aside tells the model something
    /// changed between requests, it is not a reason to keep working after
    /// the model has already decided to stop. omp's `aside` channel
    /// (`getAsideMessages`) is the reference: never abort in-flight tools,
    /// never wake an idle loop.
    pub get_aside: Option<&'a dyn Fn() -> Vec<Message>>,
    /// Pull follow-up messages (async job delivery). Checked when the
    /// model stops calling tools — a non-empty pull extends the run (the
    /// drained messages enter the context and the loop makes another
    /// round) instead of ending it. `TurnEnd` fires only when both the
    /// model stopped and the queue is empty.
    pub get_follow_up: Option<&'a dyn Fn() -> Vec<Message>>,
    /// Bare-continue decision (freebuff `task_completed` shape): when the
    /// model stops calling tools and the follow-up pull is empty, the host
    /// may still force one more LLM round with the context unchanged — no
    /// message is recorded, the model is simply called again. Used as the
    /// guard for explicit-completion tools: the run may end only after the
    /// model marks the task done; a silent stop without the mark costs at
    /// most this one extra round, then the run ends unmarked. `None`
    /// (default) keeps the pre-hook behavior: empty follow-ups end the run.
    pub should_continue: Option<&'a dyn Fn() -> bool>,
    /// Detects explicit-completion marker tools (freebuff `task_completed`
    /// shape, e.g. `mark_done`): when the model calls one, the run ends the
    /// moment that tool's result is recorded — no further LLM round-trip is
    /// owed, unlike a bare stop which the host may still continue via
    /// [`Self::should_continue`]. `None` (default) disables the marker: the
    /// run ends only through the bare-stop / error / abort / cap paths.
    pub completion_tool: Option<&'a dyn Fn(&str) -> bool>,
    /// Working directory whose ancestor chain is searched for `AGENTS.md`
    /// workspace instructions when the caller left `Context.system_prompt`
    /// unset (harness `instruction` crate, called directly here — plugin /
    /// service registration is G5). Rendered fragments are prefixed before
    /// the main-agent profile via [`build_system_prompt`]; an explicit
    /// caller prompt is never overwritten.
    ///
    /// `None` (default) skips the lookup entirely: the loop reads no ambient
    /// process state, so hosts opt in by handing over the session's cwd —
    /// the same "every policy knob arrives through this struct" contract as
    /// the other knobs.
    pub instruction_cwd: Option<&'a Path>,
}

/// Conservative turn ceiling for callers that don't set one. A real
/// tool-loop rarely exceeds ~20 round-trips; 64 leaves headroom without
/// letting a runaway model burn a provider budget silently.
pub const DEFAULT_MAX_TURNS: usize = 64;

/// Cap on consecutive rounds that fail with the *same* stream error
/// before the loop gives up and emits `TurnEnd { Error }`.
/// ponytail: a constant until someone needs per-workspace tuning —
/// promote to `LoopConfig` then (same precedent as `runner::MAX_ATTEMPTS`).
pub const MAX_SAME_ERROR: u32 = 10;

impl Default for LoopConfig<'_> {
    fn default() -> Self {
        LoopConfig {
            completion_tool: None,
            context_log: None,
            max_turns: DEFAULT_MAX_TURNS,
            maintain: None,
            get_steering: None,
            get_aside: None,
            get_follow_up: None,
            should_continue: None,
            instruction_cwd: None,
        }
    }
}

// ===== context JSONL persistence =====

/// Errors from the append-only context log.
#[derive(Debug)]
pub enum LogError {
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl std::fmt::Display for LogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogError::Io(e) => write!(f, "IO error: {e}"),
            LogError::Json(e) => write!(f, "JSON error: {e}"),
        }
    }
}

impl std::error::Error for LogError {}

/// Append-only JSONL log of context messages (one serialized Message per
/// line), mirroring the task store's fcntl-lock append pattern.
///
/// Deliberately NOT the runner.rs events.jsonl shape (#48): that evidence
/// log bounds fields at 1KB and degrades silently. A context log must
/// round-trip losslessly — a truncated message cannot be replayed into
/// the API — so records are unbounded and every write fsyncs.
pub struct ContextLog {
    path: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct LoggedMessage {
    message: Message,
}

impl ContextLog {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        ContextLog { path: path.into() }
    }

    /// Public append for callers that add messages outside the loop
    /// (e.g. the user prompt before calling run_agent).
    pub fn append_message(&self, message: &Message) -> Result<(), LogError> {
        self.append(message)
    }

    /// Append one message line. Lock held during write + fsync.
    fn append(&self, message: &Message) -> Result<(), LogError> {
        use fs2::FileExt;
        use std::io::Write;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(LogError::Io)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(LogError::Io)?;
        file.lock_exclusive().map_err(LogError::Io)?;
        let line = serde_json::to_string(&LoggedMessage {
            message: message.clone(),
        })
        .map_err(LogError::Json)?;
        file.write_all(line.as_bytes()).map_err(LogError::Io)?;
        file.write_all(b"\n").map_err(LogError::Io)?;
        file.flush().map_err(LogError::Io)?;
        file.sync_all().map_err(LogError::Io)?;
        Ok(())
    }

    /// Replay the full conversation from a log.
    pub fn load(path: impl AsRef<Path>) -> Result<Vec<Message>, LogError> {
        let text = std::fs::read_to_string(path).map_err(LogError::Io)?;
        text.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                serde_json::from_str::<LoggedMessage>(l)
                    .map(|m| m.message)
                    .map_err(LogError::Json)
            })
            .collect()
    }
}

// ===== compaction =====
// Policy (char-budget window, tool-pairing invariant, kept-window guard,
// transcript rendering) lives in `omenic-harness-compaction` (C4). What
// remains on the orbit path is the call seam — the LLM bridge and the
// wire<->DTO re-typing — isolated in [`compaction_bridge`] so this module
// stays under the R3 <= 20 line budget. A host resolves a `CharBudgetPolicy`
// out of the assembled container and hands it to the seam; passing
// `&CharBudgetPolicy::default()` is the historic hardcoded budget/region.

mod compaction_bridge;

mod fallback;

pub use compaction_bridge::{
    LlmSummarizer, compact_context, compact_context_with, message_chars, select_compaction_cut,
};

pub use fallback::{LlmProvider, WaterfallLlm};

// ===== workspace instructions (WP-A) =====
// Direct call into the harness `instruction` crate: discover `AGENTS.md` up
// the ancestor chain, render digest-deduped fragments, prefix them before
// the role profile. Plugin / service registration of the same crate is G5;
// this path needs no container and stays inside the loop's "host passes
// every knob in, loop pulls no ambient state" contract.

use crate::instruction::{InstructionCache, render_fragments};

/// Compose the default system prompt: [`prompts::agents::TASK`] prefixed with
/// the `AGENTS.md` workspace instructions discovered up the directory chain
/// from `cwd`.
///
/// Mirrors dsh's baseline rendering (`agent-instructions/render.ts`: one
/// `Instructions from: <path>` section per file, broadest first, joined into a
/// single block, same-content files collapsed by digest) minus dsh's byte
/// budget and `<system-reminder>` framing — omenic composes fragments at the
/// role layer (`infra/prompts` doc note) and the whole block ships as plain
/// system-prompt text here.
///
/// Degradation is total and silent: `cwd: None` (host didn't opt in), a chain
/// without instruction files, or any unreadable/missing file falls back to the
/// bare TASK profile. The instruction crate's lookup→load→render path is
/// infallible by construction (missing metadata and read errors are `None`,
/// never `Err`), so a run can never fail because a workspace has no
/// `AGENTS.md`.
pub fn build_system_prompt(cwd: Option<&Path>) -> String {
    // No host-supplied cwd → no discovery at all: pure role profile.
    let Some(cwd) = cwd else {
        return prompts::agents::TASK.to_string();
    };
    // Fresh cache per call: a prompt build is one-shot, and mtime gating only
    // pays for a host refreshing fragments mid-session (the plugin path, G5).
    let fragments = render_fragments(&InstructionCache::default().load(cwd));
    if fragments.is_empty() {
        return prompts::agents::TASK.to_string();
    }
    let mut prompt = String::new();
    for fragment in &fragments {
        prompt.push_str(&fragment.body);
        prompt.push_str("\n\n");
    }
    prompt.push_str(prompts::agents::TASK);
    prompt
}

// ===== agent loop =====

fn turn_stop(reason: StopReason) -> TurnStop {
    match reason {
        StopReason::EndTurn | StopReason::ToolUse => TurnStop::EndTurn,
        StopReason::MaxTokens => TurnStop::MaxTokens,
        StopReason::Aborted => TurnStop::Aborted,
    }
}

/// Run the agent loop until the model stops calling tools or the run is
/// interrupted. Collects all events (no live streaming consumer yet).
///
/// Appends each assistant/tool_result message to `context_log` when given.
/// Append a message to the context and the optional evidence log.
fn record(context: &mut Context, log: Option<&ContextLog>, msg: Message) {
    if let Some(log) = log {
        // evidence log: best-effort, never breaks the loop
        let _ = log.append(&msg);
    }
    context.messages.push(msg);
}

/// Run the agent loop, forwarding every event to `emit` as it happens
/// (live text deltas, tool calls/results). Returns when the turn ends.
///
/// The LLM round-trip is driven through `stream_cb` — deltas are forwarded
/// the moment the backend produces them, not after the whole turn buffers.
pub fn run_agent_streaming(
    backend: &dyn LlmBackend,
    model: &Model,
    context: &mut Context,
    tools: &[Box<dyn Tool>],
    signal: &AtomicBool,
    config: LoopConfig<'_>,
    emit: &mut dyn FnMut(AgentEvent),
) {
    let tool_defs: Vec<ToolDef> = tools.iter().map(|t| def(t.as_ref())).collect();
    // ponytail: linear name lookup — four builtin tools; index if the registry grows.
    let find_tool = |name: &str| tools.iter().find(|t| t.name() == name);

    // Default the system prompt to the main-agent profile, lifted
    // verbatim from `crates/prompts/prompts/agents/main.md` (which in
    // turn copies `oh-my-pi`'s `prompts/agents/task.md`). The whole file
    // — frontmatter included — is the prompt; we do not concatenate a
    // tool table here (omp does not). Filling once at entry keeps the
    // caller-provided prompt contract: explicit wins, default fills.
    // When the host hands over an instruction cwd, any `AGENTS.md` found
    // up its ancestor chain is rendered (deduped) and prefixed before the
    // profile — see [`build_system_prompt`].
    if context.system_prompt.is_none() {
        context.system_prompt = Some(build_system_prompt(config.instruction_cwd));
    }

    let mut turns_used = 0usize;
    // Consecutive identical stream errors before the loop gives up; a
    // different error resets the streak, a successful round clears it.
    let mut last_stream_error: Option<String> = None;
    let mut same_error_streak = 0u32;
    loop {
        turns_used += 1;
        if turns_used > config.max_turns {
            emit(AgentEvent::TurnEnd {
                stop_reason: TurnStop::MaxTurns,
            });
            return;
        }

        // 0. Host context maintenance before the next call (compaction etc.).
        if let Some(maintain) = config.maintain {
            maintain(backend, model, context, signal);
        }

        // 0.5. Drain steering queued since the last round (omp pulls
        // steering at the inner-loop top, before the provider call).
        if let Some(get_steering) = config.get_steering {
            for msg in get_steering() {
                record(context, config.context_log, msg);
            }
        }
        // 0.6. Asides land at the same boundary but never keep the loop
        // alive on their own — see `LoopConfig::get_aside`.
        if let Some(get_aside) = config.get_aside {
            for msg in get_aside() {
                record(context, config.context_log, msg);
            }
        }
        emit(AgentEvent::TurnStart);

        // 1. Stream one LLM turn, forwarding deltas live as they arrive.
        let mut text = String::new();
        let mut stop_reason = StopReason::EndTurn;
        let mut tool_calls: Vec<protocol::events::ToolCallSpec> = Vec::new();
        let mut stream_failed: Option<String> = None;

        backend.stream_cb(model, context, &tool_defs, signal, &mut |ev| match ev {
            StreamEvent::TextDelta(delta) => {
                text.push_str(delta);
                // The clone keeps AgentEvent lifetime-free (consumers store
                // the event in channels and state); one small alloc per
                // chunk is the price — a Cow<'a, str> would thread a
                // lifetime through every AgentEvent consumer.
                emit(AgentEvent::AssistantText {
                    delta: delta.clone(),
                });
            }
            StreamEvent::ReasoningDelta(delta) => {
                emit(AgentEvent::AssistantReasoning {
                    delta: delta.clone(),
                });
            }
            StreamEvent::ToolCall(tc) => {
                emit(AgentEvent::ToolCall(tc.clone()));
                tool_calls.push(tc.clone());
            }
            StreamEvent::Done { stop_reason: r } => stop_reason = *r,
            StreamEvent::Error(msg) => stream_failed = Some(msg.clone()),
        });

        if let Some(err) = stream_failed {
            // Invariant 3 analog: record assistant text without dangling calls.
            let msg = Message::assistant(text, &[]);
            record(context, config.context_log, msg);
            // Auth failures (401/403) fail identically on every retry —
            // re-driving just burns rounds: hard stop, no continuation.
            if llm::openai::is_auth_failure(&err) {
                emit(AgentEvent::TurnEnd {
                    stop_reason: TurnStop::Error,
                });
                return;
            }
            let streak_reset = last_stream_error.as_deref() != Some(err.as_str());
            if streak_reset {
                same_error_streak = 0;
            }
            last_stream_error = Some(err.clone());
            if same_error_streak >= MAX_SAME_ERROR {
                emit(AgentEvent::TurnEnd {
                    stop_reason: TurnStop::Error,
                });
                return;
            }
            same_error_streak += 1;
            // ponytail: bare re-round (freebuff task_completed shape) — no
            // error note is injected; the streak cap above bounds the burn
            // of a deterministic failure.
            continue;
        }

        // A clean round clears the error streak.
        last_stream_error = None;
        same_error_streak = 0;

        // 3. Abort mid-stream (invariant 3): record the assistant message
        // WITHOUT tool_use blocks — no results will follow, and a restored
        // session must never trip the API's pairing constraint.
        if stop_reason == StopReason::Aborted {
            let msg = Message::assistant(text, &[]);
            record(context, config.context_log, msg);
            emit(AgentEvent::TurnEnd {
                stop_reason: TurnStop::Aborted,
            });
            return;
        }

        // 2. Backfill the assistant reply.
        let assistant_msg = Message::assistant(text, &tool_calls);
        record(context, config.context_log, assistant_msg);

        // 4. Truncated args must not execute (invariant 2): backfill errors instead.
        if stop_reason == StopReason::MaxTokens && !tool_calls.is_empty() {
            let results: Vec<(String, String)> = tool_calls
                .iter()
                .map(|tc| {
                    (
                        tc.id.clone(),
                        format!(
                            "error: output truncated by max_tokens, tool \"{}\" args may be incomplete.",
                            tc.name
                        ),
                    )
                })
                .collect();
            for ((id, content), tc) in results.iter().zip(&tool_calls) {
                emit(AgentEvent::ToolResult {
                    id: id.clone(),
                    name: tc.name.clone(),
                    result: content.clone(),
                });
            }
            let msg = Message::tool_results(&results);
            record(context, config.context_log, msg);
            continue;
        }

        // 5. No tool calls → model wants to stop. Follow-ups (async job
        // delivery) extend the run; with an empty queue the turn ends for
        // real. A tool_use stop without call deltas is malformed; treat it
        // as a clean end of turn like llm.ts does.
        if tool_calls.is_empty() {
            let follow_ups = config.get_follow_up.map(|get| get()).unwrap_or_default();
            if follow_ups.is_empty() {
                // Bare-continue guard: the host may force one more round
                // with the context unchanged (explicit-completion tools —
                // see `LoopConfig::should_continue`).
                if config.should_continue.map(|f| f()).unwrap_or(false) {
                    continue;
                }
                emit(AgentEvent::TurnEnd {
                    stop_reason: turn_stop(stop_reason),
                });
                return;
            }
            for msg in follow_ups {
                record(context, config.context_log, msg);
            }
            continue;
        }

        // 6. Execute serially; unknown tools and panics-free errors become error strings.
        let mut results: Vec<(String, String)> = Vec::with_capacity(tool_calls.len());
        for tc in &tool_calls {
            if signal.load(Ordering::Relaxed) {
                break;
            }
            emit(AgentEvent::ToolStart {
                id: tc.id.clone(),
                name: tc.name.clone(),
            });
            let result = match find_tool(&tc.name) {
                None => Err(tools::ToolError::Message(format!(
                    "tool \"{}\" not found",
                    tc.name
                ))),
                Some(tool) => tool.execute(&tc.args, signal),
            };
            let content = match result {
                Ok(s) => s,
                Err(e) => format!("error: {e}"),
            };
            emit(AgentEvent::ToolResult {
                id: tc.id.clone(),
                name: tc.name.clone(),
                result: content.clone(),
            });
            results.push((tc.id.clone(), content));
        }

        // 7. Invariant 1: every remaining tool_call still gets its tool_result.
        for tc in &tool_calls[results.len()..] {
            emit(AgentEvent::ToolResult {
                id: tc.id.clone(),
                name: tc.name.clone(),
                result: "error: aborted".into(),
            });
            results.push((tc.id.clone(), "error: aborted".into()));
        }

        let msg = Message::tool_results(&results);
        record(context, config.context_log, msg);

        // 8. Explicit completion: a marker tool (e.g. mark_done) ends the
        // run here — the model confirmed the task is done, so no further
        // LLM round is owed (unlike a bare stop the host may continue).
        let marked = tool_calls
            .iter()
            .any(|tc| config.completion_tool.map_or(false, |f| f(&tc.name)));
        if marked {
            emit(AgentEvent::TurnEnd {
                stop_reason: TurnStop::EndTurn,
            });
            return;
        }
    }
}

/// Run the agent loop and collect all events. Convenience wrapper over
/// [`run_agent_streaming`] for callers without a live consumer. Uses the
/// default compaction policy; hosts with custom maintenance should call
/// [`run_agent_streaming`] directly.
pub fn run_agent(
    backend: &dyn LlmBackend,
    model: &Model,
    context: &mut Context,
    tools: &[Box<dyn Tool>],
    signal: &AtomicBool,
    context_log: Option<&ContextLog>,
) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    // The default policy carries the historic budget/region
    // (COMPACT_CHAR_BUDGET + default RegionBudget); the seam streams the
    // summary through the same backend as the loop, so this is identical to
    // the pre-G6 hardcoded compact_context.
    let maintain = |b: &dyn LlmBackend, m: &Model, c: &mut Context, s: &AtomicBool| {
        compact_context(b, m, c, s, context_log)
    };
    run_agent_streaming(
        backend,
        model,
        context,
        tools,
        signal,
        LoopConfig {
            context_log,
            maintain: Some(&maintain),
            ..LoopConfig::default()
        },
        &mut |e| {
            events.push(e);
        },
    );
    events
}
