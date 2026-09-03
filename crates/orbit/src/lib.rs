//! Agent loop: stream → tool_calls → execute → backfill → repeat.
//!
//! Port of pi-from-scratch `src/agent.ts`. The four invariants:
//! 1. Every tool_call gets a matching tool_result (API hard constraint).
//! 2. max_tokens truncation → tools NOT executed; error results backfilled
//!    so the model resends complete args.
//! 3. Abort → pending tool_calls dropped and the assistant message recorded
//!    without them, so a restored session never trips the API.
//! 4. Compaction failure → original context kept untouched.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use adaptor::{Context, Message, Model, StopReason, StreamEvent, ToolCallSpec, ToolDef};
use tools::{Tool, def};

/// Compaction triggers when the estimated context size exceeds this many
/// characters (~4 chars/token, so roughly 30k tokens).
/// ponytail: fixed budget — `Model` carries no context-window field; derive
/// it from provider metadata once one exists.
const COMPACT_CHAR_BUDGET: usize = 120_000;
/// Characters of the newest messages kept verbatim during compaction.
const KEEP_RECENT_CHARS: usize = 30_000;
/// Newest messages always kept verbatim, even when oversized on their own.
const KEEP_RECENT_MIN: usize = 2;

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
        adaptor::openai::stream_cb(model, context, tools, signal, emit)
    }
}

/// Events emitted by the agent loop, for UI/evidence consumption.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    AssistantText {
        delta: String,
    },
    ToolCall(ToolCallSpec),
    ToolResult {
        id: String,
        name: String,
        result: String,
    },
    TurnEnd {
        stop_reason: TurnStop,
    },
}

/// Loop-level stop reasons (`error` added on top of the stream set).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStop {
    EndTurn,
    MaxTokens,
    Aborted,
    Error,
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

/// Estimated size of one message in characters: text measured directly,
/// block content through its JSON encoding (what the wire actually carries).
/// ponytail: role/framing overhead uncounted — a rounding error next to
/// message bodies.
fn message_chars(m: &Message) -> usize {
    match &m.content {
        adaptor::Content::Text(s) => s.len(),
        blocks => serde_json::to_string(blocks).unwrap_or_default().len(),
    }
}

/// Estimated size of the whole context, system prompt included.
fn context_chars(context: &Context) -> usize {
    context.system_prompt.as_ref().map_or(0, |s| s.len())
        + context.messages.iter().map(message_chars).sum::<usize>()
}

/// A user message carrying only tool results. Keeping it without the
/// assistant `tool_use` that precedes it would break invariant 1.
fn is_tool_result_only(m: &Message) -> bool {
    match &m.content {
        adaptor::Content::Blocks(bs) => {
            !bs.is_empty()
                && bs
                    .iter()
                    .all(|b| matches!(b, adaptor::Block::ToolResult { .. }))
        }
        _ => false,
    }
}

/// First index to keep verbatim: walks newest → oldest spending `budget`
/// characters, always keeping at least [`KEEP_RECENT_MIN`] messages. `0`
/// means the whole context fits the recent window — nothing to compact.
fn select_compaction_cut(messages: &[Message], budget: usize) -> usize {
    let mut used = 0usize;
    let mut cut = messages.len();
    for (i, m) in messages.iter().enumerate().rev() {
        let size = message_chars(m);
        if used + size > budget && messages.len() - i > KEEP_RECENT_MIN {
            break;
        }
        used += size;
        cut = i;
    }
    if cut == 0 {
        return 0;
    }
    // Invariant 1: the kept window must not start on orphan tool_results
    // whose tool_use blocks are being summarized away.
    while cut < messages.len() && is_tool_result_only(&messages[cut]) {
        cut += 1;
    }
    cut
}

/// Summarize old messages when the context grows too large.
/// Invariant 4: on any failure the context is left untouched.
fn compact_context(
    backend: &dyn LlmBackend,
    model: &Model,
    context: &mut Context,
    signal: &AtomicBool,
) {
    if signal.load(Ordering::Relaxed) || context_chars(context) < COMPACT_CHAR_BUDGET {
        return;
    }

    let keep_at = select_compaction_cut(&context.messages, KEEP_RECENT_CHARS);
    if keep_at == 0 {
        return; // nothing older than the recent window — leave the context alone
    }
    // The newest KEEP_RECENT_MIN messages are kept whatever their size. When
    // they alone blow the budget, summarizing the prefix cannot get under it:
    // shipping a summary here would just re-summarize the previous summary
    // every turn. Leave the context intact instead.
    let kept_chars: usize = context.system_prompt.as_ref().map_or(0, |s| s.len())
        + context.messages[keep_at..]
            .iter()
            .map(message_chars)
            .sum::<usize>();
    if kept_chars >= COMPACT_CHAR_BUDGET {
        return;
    }
    let old = &context.messages[..keep_at];
    let recent: Vec<Message> = context.messages[keep_at..].to_vec();

    let conversation: String = old
        .iter()
        .map(|m| {
            format!(
                "{:?}: {}",
                m.role,
                match &m.content {
                    adaptor::Content::Text(s) => s.clone(),
                    blocks => serde_json::to_string(blocks).unwrap_or_default(),
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let summary_context = Context {
        system_prompt: Some(
            "请将以下对话总结为简洁的上下文摘要，保留关键决策、已做的工作和待办事项。".into(),
        ),
        messages: vec![Message::user_text(conversation)],
    };

    let mut summary = String::new();
    let mut failed = false;
    for ev in backend.stream(model, &summary_context, &[], signal) {
        match ev {
            StreamEvent::TextDelta(delta) => summary.push_str(&delta),
            StreamEvent::Done {
                stop_reason: StopReason::Aborted,
            } => {
                failed = true;
                break;
            }
            StreamEvent::Error(_) => {
                failed = true;
                break;
            }
            _ => {}
        }
    }

    if failed || summary.is_empty() {
        return; // invariant 4: keep original messages over a broken summary
    }

    let mut replaced = vec![Message::user_text(format!("[context summary]\n{summary}"))];
    replaced.extend(recent);
    context.messages = replaced;
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
pub fn run_agent_streaming(
    backend: &dyn LlmBackend,
    model: &Model,
    context: &mut Context,
    tools: &[Box<dyn Tool>],
    signal: &AtomicBool,
    context_log: Option<&ContextLog>,
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
    if context.system_prompt.is_none() {
        context.system_prompt = Some(prompts::agents::MAIN.to_string());
    }

    loop {
        // 0. Compact oversized contexts before the next call.
        compact_context(backend, model, context, signal);

        // 1. Stream one LLM turn, collecting text + tool calls.
        let mut text = String::new();
        let mut stop_reason = StopReason::EndTurn;
        let mut tool_calls: Vec<ToolCallSpec> = Vec::new();

        for ev in backend.stream(model, context, &tool_defs, signal) {
            match ev {
                StreamEvent::TextDelta(delta) => {
                    text.push_str(&delta);
                    emit(AgentEvent::AssistantText { delta });
                }
                StreamEvent::ToolCall(tc) => {
                    emit(AgentEvent::ToolCall(tc.clone()));
                    tool_calls.push(tc);
                }
                StreamEvent::Done { stop_reason: r } => stop_reason = r,
                StreamEvent::Error(e) => {
                    // Invariant 3 analog: record assistant text without dangling calls.
                    let msg = Message::assistant(text, &[]);
                    record(context, context_log, msg);
                    emit(AgentEvent::TurnEnd {
                        stop_reason: TurnStop::Error,
                    });
                    let _ = e;
                    return;
                }
            }
        }

        // 3. Abort mid-stream (invariant 3): record the assistant message
        // WITHOUT tool_use blocks — no results will follow, and a restored
        // session must never trip the API's pairing constraint.
        if stop_reason == StopReason::Aborted {
            let msg = Message::assistant(text, &[]);
            record(context, context_log, msg);
            emit(AgentEvent::TurnEnd {
                stop_reason: TurnStop::Aborted,
            });
            return;
        }

        // 2. Backfill the assistant reply.
        let assistant_msg = Message::assistant(text, &tool_calls);
        record(context, context_log, assistant_msg);

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
            record(context, context_log, msg);
            continue;
        }

        // 5. No tool calls → done. A tool_use stop without call deltas is malformed;
        // treat it as a clean end of turn like llm.ts does.
        if tool_calls.is_empty() {
            emit(AgentEvent::TurnEnd {
                stop_reason: turn_stop(stop_reason),
            });
            return;
        }

        // 6. Execute serially; unknown tools and panics-free errors become error strings.
        let mut results: Vec<(String, String)> = Vec::with_capacity(tool_calls.len());
        for tc in &tool_calls {
            if signal.load(Ordering::Relaxed) {
                break;
            }
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
        record(context, context_log, msg);
    }
}

/// Run the agent loop and collect all events. Convenience wrapper over
/// [`run_agent_streaming`] for callers without a live consumer.
pub fn run_agent(
    backend: &dyn LlmBackend,
    model: &Model,
    context: &mut Context,
    tools: &[Box<dyn Tool>],
    signal: &AtomicBool,
    context_log: Option<&ContextLog>,
) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    run_agent_streaming(
        backend,
        model,
        context,
        tools,
        signal,
        context_log,
        &mut |e| {
            events.push(e);
        },
    );
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use adaptor::Content;
    use serde_json::json;

    fn model() -> Model {
        Model {
            api_key: "k".into(),
            model: "test".into(),
            base_url: None,
            max_tokens: None,
        }
    }

    fn sig() -> AtomicBool {
        AtomicBool::new(false)
    }

    fn call(id: &str, name: &str) -> StreamEvent {
        StreamEvent::ToolCall(ToolCallSpec {
            id: id.into(),
            name: name.into(),
            args: json!({}),
        })
    }

    struct EchoTool;

    impl Tool for EchoTool {
        fn name(&self) -> &'static str {
            "echo_tool"
        }
        fn description(&self) -> String {
            "echoes".into()
        }
        fn parameters(&self) -> serde_json::Value {
            json!({"type": "object"})
        }
        fn execute(
            &self,
            _args: &serde_json::Value,
            signal: &AtomicBool,
        ) -> Result<String, tools::ToolError> {
            if signal.load(Ordering::Relaxed) {
                return Ok("aborted".into());
            }
            Ok("echo!".into())
        }
    }

    /// Trait takes `&self`; tests mutate through RefCell.
    struct Shared(std::cell::RefCell<Scripted>);
    impl LlmBackend for Shared {
        fn stream_cb(
            &self,
            _model: &Model,
            context: &Context,
            _tools: &[ToolDef],
            _signal: &AtomicBool,
            emit: &mut dyn FnMut(&StreamEvent),
        ) {
            let s = &mut *self.0.borrow_mut();
            s.seen_contexts.push(context.clone());
            let t = match s.turns.get(s.calls_made) {
                Some(t) => t.clone(),
                // Turns exhausted: end cleanly so the loop terminates.
                None => vec![StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                }],
            };
            s.calls_made += 1;
            for ev in &t {
                emit(ev);
            }
        }
    }

    #[test]
    fn plain_answer_ends_loop() {
        let backend = Shared(std::cell::RefCell::new(Scripted::new(vec![vec![
            StreamEvent::TextDelta("hello ".into()),
            StreamEvent::TextDelta("world".into()),
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            },
        ]])));
        let mut ctx = Context::default();
        ctx.system_prompt = Some("sys".into());
        let events = run_agent(&backend, &model(), &mut ctx, &[], &sig(), None);

        assert_eq!(
            events,
            vec![
                AgentEvent::AssistantText {
                    delta: "hello ".into()
                },
                AgentEvent::AssistantText {
                    delta: "world".into()
                },
                AgentEvent::TurnEnd {
                    stop_reason: TurnStop::EndTurn
                },
            ]
        );
        assert_eq!(ctx.messages.len(), 1);
        assert_eq!(
            ctx.messages[0],
            Message::assistant("hello world".into(), &[])
        );
    }

    #[test]
    fn tool_call_executes_and_result_is_backfilled() {
        let backend = Shared(std::cell::RefCell::new(Scripted::new(vec![
            vec![
                call("t1", "echo_tool"),
                StreamEvent::Done {
                    stop_reason: StopReason::ToolUse,
                },
            ],
            vec![
                StreamEvent::TextDelta("done".into()),
                StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                },
            ],
        ])));
        let tools: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];
        let mut ctx = Context::default();
        let events = run_agent(&backend, &model(), &mut ctx, &tools, &sig(), None);

        assert!(events.contains(&AgentEvent::ToolResult {
            id: "t1".into(),
            name: "echo_tool".into(),
            result: "echo!".into(),
        }));
        // assistant(tool_use) then user(tool_result) then assistant(final).
        assert_eq!(ctx.messages.len(), 3);
        assert_eq!(
            ctx.messages[1],
            Message::tool_results(&[("t1".into(), "echo!".into())])
        );
    }

    #[test]
    fn invariant_2_max_tokens_truncation_skips_execution() {
        let backend = Shared(std::cell::RefCell::new(Scripted::new(vec![
            vec![
                call("t1", "echo_tool"),
                StreamEvent::Done {
                    stop_reason: StopReason::MaxTokens,
                },
            ],
            // Retry turn succeeds.
            vec![StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            }],
        ])));
        let tools: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];
        let mut ctx = Context::default();
        let events = run_agent(&backend, &model(), &mut ctx, &tools, &sig(), None);

        assert!(events.contains(&AgentEvent::ToolResult {
            id: "t1".into(),
            name: "echo_tool".into(),
            result: "error: output truncated by max_tokens, tool \"echo_tool\" args may be incomplete.".into(),
        }));
        // The tool never executed; the retry saw exactly one backfilled user message.
        assert_eq!(ctx.messages.len(), 3);
        assert_eq!(
            ctx.messages[1],
            Message::tool_results(&[(
                "t1".into(),
                "error: output truncated by max_tokens, tool \"echo_tool\" args may be incomplete."
                    .into()
            )])
        );
    }

    #[test]
    fn invariant_3_abort_drops_pending_tool_calls() {
        let backend = Shared(std::cell::RefCell::new(Scripted::new(vec![vec![
            StreamEvent::TextDelta("partial".into()),
            call("t1", "echo_tool"),
            StreamEvent::Done {
                stop_reason: StopReason::Aborted,
            },
        ]])));
        let tools: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];
        let mut ctx = Context::default();
        let events = run_agent(&backend, &model(), &mut ctx, &tools, &sig(), None);

        assert_eq!(
            events.last(),
            Some(&AgentEvent::TurnEnd {
                stop_reason: TurnStop::Aborted
            })
        );
        // Assistant message recorded WITHOUT the tool_use block.
        assert_eq!(ctx.messages.len(), 1);
        assert_eq!(ctx.messages[0], Message::assistant("partial".into(), &[]));
    }

    #[test]
    fn invariant_1_abort_mid_execution_still_backfills_every_result() {
        /// First execute succeeds and flips the shared signal, so the loop
        /// breaks before reaching t2 — t2 must still get a result.
        struct FlipOnSecond<'a>(&'a AtomicBool, AtomicBool);
        impl Tool for FlipOnSecond<'_> {
            fn name(&self) -> &'static str {
                "echo_tool"
            }
            fn description(&self) -> String {
                "echoes".into()
            }
            fn parameters(&self) -> serde_json::Value {
                json!({"type": "object"})
            }
            fn execute(
                &self,
                _args: &serde_json::Value,
                _signal: &AtomicBool,
            ) -> Result<String, tools::ToolError> {
                let _ = self.1.load(Ordering::Relaxed);
                self.0.store(true, Ordering::Relaxed);
                Ok("echo!".into())
            }
        }

        let backend = Shared(std::cell::RefCell::new(Scripted::new(vec![vec![
            call("t1", "echo_tool"),
            call("t2", "echo_tool"),
            StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
            },
        ]])));
        // Leak so the tool satisfies Box<dyn Tool>'s 'static bound.
        let signal: &'static AtomicBool = Box::leak(Box::new(AtomicBool::new(false)));
        let tools: Vec<Box<dyn Tool>> =
            vec![Box::new(FlipOnSecond(signal, AtomicBool::new(false)))];
        let mut ctx = Context::default();
        let events = run_agent(&backend, &model(), &mut ctx, &tools, &signal, None);

        let results: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::ToolResult { id, result, .. } => Some((id.as_str(), result.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(results, [("t1", "echo!"), ("t2", "error: aborted")]);
    }

    /// A user message of exactly 1000 chars, tagged by index so a retained
    /// window is identifiable. 1000 divides the char budgets evenly, which
    /// makes every cut in these tests exact rather than approximate.
    fn filler(tag: usize) -> Message {
        Message::user_text(format!("{tag:04}{}", "x".repeat(996)))
    }

    fn bulk(n: usize) -> Vec<Message> {
        (0..n).map(filler).collect()
    }

    #[test]
    fn compaction_cut_spends_the_recent_char_budget() {
        let msgs = bulk(200);
        // 1000 chars per message: a 30_000-char window is the newest 30.
        assert_eq!(select_compaction_cut(&msgs, 30_000), 170);
        // 2_500 buys two whole messages; the third would overflow.
        assert_eq!(select_compaction_cut(&msgs, 2_500), 198);
        // Whole context fits the window → nothing eligible to compact.
        assert_eq!(select_compaction_cut(&msgs, 1_000_000), 0);
        // Newest messages larger than the budget still keep the floor.
        assert_eq!(
            select_compaction_cut(&msgs, 0),
            msgs.len() - KEEP_RECENT_MIN
        );
        // Degenerate inputs stay no-ops instead of panicking.
        assert_eq!(select_compaction_cut(&[], 30_000), 0);
        assert_eq!(select_compaction_cut(&msgs[..1], 0), 0);
    }

    #[test]
    fn compaction_cut_skips_orphan_tool_results() {
        // Invariant 1: a kept window may not start on tool_results whose
        // tool_use blocks are about to be summarized away.
        let calls = [ToolCallSpec {
            id: "t1".into(),
            name: "echo_tool".into(),
            args: json!({}),
        }];
        let msgs = vec![
            filler(0),
            Message::assistant("thinking".into(), &calls),
            Message::tool_results(&[("t1".into(), "done".into())]),
            filler(3),
            filler(4),
        ];
        // Budget buys the two fillers plus exactly the tool_results message,
        // so the raw cut lands on index 2 and must advance to 3.
        let budget = 2_000 + message_chars(&msgs[2]);
        assert_eq!(select_compaction_cut(&msgs, budget), 3);
    }

    #[test]
    fn compaction_skipped_below_char_threshold() {
        let backend = Shared(std::cell::RefCell::new(Scripted::new(vec![vec![
            StreamEvent::TextDelta("hi".into()),
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            },
        ]])));
        // 100 messages = 100_000 chars: far past the old 50-message trigger,
        // still under the char budget → no summary call at all.
        // Caller-provided prompt must win — see run_agent_streaming contract:
        // only fills system_prompt when caller left it None.
        let caller_prompt = "test caller prompt";
        let mut ctx = Context {
            system_prompt: Some(caller_prompt.into()),
            messages: bulk(100),
        };
        let before = ctx.messages.clone();
        let _ = run_agent(&backend, &model(), &mut ctx, &[], &sig(), None);

        let seen = backend.0.borrow();
        assert_eq!(seen.seen_contexts.len(), 1, "summary stream was issued");
        assert_eq!(
            seen.seen_contexts[0].system_prompt.as_deref(),
            Some(caller_prompt),
            "caller-provided prompt must not be overwritten by run_agent_streaming"
        );
        assert_eq!(ctx.messages.len(), before.len() + 1);
        assert!(ctx.messages.iter().take(before.len()).eq(before.iter()));
    }

    #[test]
    fn invariant_4_compaction_failure_keeps_context() {
        // First call = oversized context triggers compaction which errors;
        // second call = the normal turn must see ALL original messages intact.
        let turns = vec![
            vec![StreamEvent::Error("summary backend down".into())],
            vec![
                StreamEvent::TextDelta("ok".into()),
                StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                },
            ],
        ];
        let backend = Shared(std::cell::RefCell::new(Scripted::new(turns)));

        // 200_000 chars: above the char budget.
        let mut ctx = Context {
            system_prompt: None,
            messages: bulk(200),
        };
        let before = ctx.clone();
        let _ = run_agent(&backend, &model(), &mut ctx, &[], &sig(), None);

        // Compaction failed → no summary message injected; originals intact.
        assert_eq!(ctx.messages.len(), before.messages.len() + 1); // + final assistant msg
        assert!(ctx.messages.iter().take(200).eq(before.messages.iter()));
    }
    #[test]
    fn run_agent_injects_default_system_prompt_when_caller_leaves_none() {
        // run_agent_streaming contract: when caller doesn't set
        // `Context.system_prompt`, orbit fills it with the main-agent
        // profile lifted from `crates/prompts/prompts/agents/main.md`
        // (oh-my-pi `prompts/agents/task.md`). The whole file —
        // frontmatter included — is the prompt; no concatenation.
        let backend = Shared(std::cell::RefCell::new(Scripted::new(vec![vec![
            StreamEvent::TextDelta("hi".into()),
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            },
        ]])));
        let mut ctx = Context {
            system_prompt: None,
            messages: vec![Message::user_text("hi")],
        };
        let tools: Vec<Box<dyn tools::Tool>> = vec![Box::new(tools::read::ReadFile)];
        let _ = run_agent_streaming(
            &backend,
            &model(),
            &mut ctx,
            &tools,
            &sig(),
            None,
            &mut |_| {},
        );

        let seen = backend.0.borrow();
        assert_eq!(seen.seen_contexts.len(), 1);
        let prompt = seen.seen_contexts[0]
            .system_prompt
            .as_deref()
            .expect("system_prompt should be filled when caller left None");
        // omp-style profile: frontmatter, <directives> block, the
        // `tools:` line in the frontmatter lists what omenic exposes.
        assert!(prompt.starts_with("---\n"));
        assert!(prompt.contains("<directives>"));
        assert!(
            prompt.contains("tools: read, edit, write, run_bash, grep, glob, delete_file, task")
        );
    }
    #[test]
    fn compaction_broken_summary_leaves_context_identical() {
        // Error, abort, and empty-summary paths are each a pure no-op.
        let failures = vec![
            vec![StreamEvent::Error("backend down".into())],
            vec![
                StreamEvent::TextDelta("partial".into()),
                StreamEvent::Done {
                    stop_reason: StopReason::Aborted,
                },
            ],
            vec![StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            }],
        ];
        for turn in failures {
            let backend = Shared(std::cell::RefCell::new(Scripted::new(vec![turn])));
            let mut ctx = Context {
                system_prompt: Some("sys".into()),
                messages: bulk(200),
            };
            let before = ctx.clone();
            compact_context(&backend, &model(), &mut ctx, &sig());
            assert_eq!(ctx, before);
        }
    }

    #[test]
    fn compaction_keeps_newest_window_on_success() {
        let turns = vec![
            vec![
                StreamEvent::TextDelta("the gist".into()),
                StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                },
            ],
            vec![StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            }],
        ];
        let backend = Shared(std::cell::RefCell::new(Scripted::new(turns)));

        let mut ctx = Context {
            system_prompt: None,
            messages: bulk(200),
        };
        let before = ctx.messages.clone();
        let _ = run_agent(&backend, &model(), &mut ctx, &[], &sig(), None);

        // [summary] + newest window + final assistant msg.
        let kept = KEEP_RECENT_CHARS / 1000;
        assert_eq!(ctx.messages.len(), 1 + kept + 1);
        assert_eq!(
            ctx.messages[0],
            Message::user_text("[context summary]\nthe gist")
        );
        assert!(
            ctx.messages[1..=kept]
                .iter()
                .eq(before[200 - kept..].iter())
        );

        // The summary request carried the old prefix and none of the window.
        let seen = backend.0.borrow();
        let sent = match &seen.seen_contexts[0].messages[0].content {
            Content::Text(s) => s.as_str(),
            other => panic!("summary request should be plain text: {other:?}"),
        };
        assert!(sent.contains("0169"), "oldest prefix must be summarized");
        assert!(!sent.contains("0170"), "kept window must not be summarized");
    }

    #[test]
    fn compaction_skipped_when_kept_window_alone_exceeds_budget() {
        // Two newest messages of 200_000 chars each: KEEP_RECENT_MIN pins them
        // in place, so no prefix summary can bring the context under budget.
        // Compacting anyway would summarize the previous summary every turn.
        let backend = Shared(std::cell::RefCell::new(Scripted::new(vec![vec![
            StreamEvent::TextDelta("ok".into()),
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            },
        ]])));
        let mut msgs = bulk(5);
        msgs.push(Message::user_text("A".repeat(200_000)));
        msgs.push(Message::user_text("B".repeat(200_000)));
        // The cut is nonzero — the guard, not `keep_at == 0`, must stop this.
        assert_eq!(
            select_compaction_cut(&msgs, KEEP_RECENT_CHARS),
            msgs.len() - KEEP_RECENT_MIN
        );

        let mut ctx = Context {
            system_prompt: None,
            messages: msgs,
        };
        let before = ctx.messages.clone();
        let _ = run_agent(&backend, &model(), &mut ctx, &[], &sig(), None);

        // Exactly one backend call: the regular turn, never a summary request.
        let seen = backend.0.borrow();
        assert_eq!(seen.seen_contexts.len(), 1, "summary stream was issued");
        assert_eq!(seen.seen_contexts[0].messages, before);
        // Originals untouched; only the final assistant reply was appended.
        assert_eq!(ctx.messages.len(), before.len() + 1);
        assert!(ctx.messages.iter().take(before.len()).eq(before.iter()));
    }

    #[test]
    fn compaction_skipped_when_system_prompt_alone_exceeds_budget() {
        let backend = Shared(std::cell::RefCell::new(Scripted::new(vec![vec![
            StreamEvent::TextDelta("summary should not be requested".into()),
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            },
        ]])));
        let mut ctx = Context {
            system_prompt: Some("s".repeat(110_000)),
            messages: vec![Message::user_text("m".repeat(25_000))],
        };
        let before = ctx.messages.clone();

        compact_context(&backend, &model(), &mut ctx, &sig());

        assert!(backend.0.borrow().seen_contexts.is_empty());
        assert_eq!(ctx.messages, before);
    }

    /// Scripted backend replays canned event lists per call.
    struct Scripted {
        turns: Vec<Vec<StreamEvent>>,
        calls_made: usize,
        seen_contexts: Vec<Context>,
    }

    impl Scripted {
        fn new(turns: Vec<Vec<StreamEvent>>) -> Self {
            Scripted {
                turns,
                calls_made: 0,
                seen_contexts: vec![],
            }
        }
    }

    #[test]
    fn context_log_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("sub/ctx.jsonl");
        let backend = Shared(std::cell::RefCell::new(Scripted::new(vec![
            vec![
                call("t1", "echo_tool"),
                StreamEvent::Done {
                    stop_reason: StopReason::ToolUse,
                },
            ],
            vec![StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            }],
        ])));
        let tools: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];
        let mut ctx = Context::default();
        let log = ContextLog::new(&log_path);
        let _ = run_agent(&backend, &model(), &mut ctx, &tools, &sig(), Some(&log));

        let replayed = ContextLog::load(&log_path).unwrap();
        assert_eq!(replayed.len(), ctx.messages.len());
        assert_eq!(replayed, ctx.messages);
    }
}
