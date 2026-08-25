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

use super::llm::{Context, Message, Model, StopReason, StreamEvent, ToolCallSpec, ToolDef};
use super::tools::{Tool, def};

/// Compaction triggers when message count exceeds this.
const COMPACT_THRESHOLD: usize = 50;
/// Recent messages kept verbatim during compaction.
const KEEP_RECENT: usize = 20;

/// LLM backend abstraction: the only seam between loop and network,
/// so invariants are testable offline with scripted streams.
pub trait LlmBackend {
    fn stream(
        &self,
        model: &Model,
        context: &Context,
        tools: &[ToolDef],
        signal: &AtomicBool,
    ) -> Vec<StreamEvent>;
}

/// Production backend: real OpenAI-compatible HTTP streaming.
pub struct HttpLlm;

impl LlmBackend for HttpLlm {
    fn stream(
        &self,
        model: &Model,
        context: &Context,
        tools: &[ToolDef],
        signal: &AtomicBool,
    ) -> Vec<StreamEvent> {
        super::llm::stream(model, context, tools, signal)
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

/// Summarize old messages when the context grows too large.
/// Invariant 4: on any failure the context is left untouched.
fn compact_context(
    backend: &dyn LlmBackend,
    model: &Model,
    context: &mut Context,
    signal: &AtomicBool,
) {
    if signal.load(Ordering::Relaxed) || context.messages.len() < COMPACT_THRESHOLD {
        return;
    }

    let keep_at = context.messages.len() - KEEP_RECENT;
    let old = &context.messages[..keep_at];
    let recent: Vec<Message> = context.messages[keep_at..].to_vec();

    let conversation: String = old
        .iter()
        .map(|m| {
            format!(
                "{:?}: {}",
                m.role,
                match &m.content {
                    super::llm::Content::Text(s) => s.clone(),
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
                None => Err(super::tools::ToolError::Message(format!(
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
    use crate::harness::llm::{Content, Role};
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
        ) -> Result<String, crate::harness::tools::ToolError> {
            if signal.load(Ordering::Relaxed) {
                return Ok("aborted".into());
            }
            Ok("echo!".into())
        }
    }

    /// Trait takes `&self`; tests mutate through RefCell.
    struct Shared(std::cell::RefCell<Scripted>);
    impl LlmBackend for Shared {
        fn stream(
            &self,
            _model: &Model,
            context: &Context,
            _tools: &[ToolDef],
            _signal: &AtomicBool,
        ) -> Vec<StreamEvent> {
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
            t
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
        struct FlipOnSecond<'a>(&'a AtomicBool, std::cell::Cell<bool>);
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
            ) -> Result<String, crate::harness::tools::ToolError> {
                let _ = self.1.get();
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
            vec![Box::new(FlipOnSecond(signal, std::cell::Cell::new(false)))];
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

        // Build a context above the threshold.
        let mut ctx = Context::default();
        for i in 0..60 {
            ctx.messages.push(Message::user_text(format!("msg{i}")));
        }
        let before = ctx.clone();
        let _ = run_agent(&backend, &model(), &mut ctx, &[], &sig(), None);

        // Compaction failed → no summary message injected; originals intact.
        assert_eq!(ctx.messages.len(), before.messages.len() + 1); // + final assistant msg
        assert!(ctx.messages.iter().take(60).eq(before.messages.iter()));
    }

    #[test]
    fn compaction_replaces_old_messages_on_success() {
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

        let mut ctx = Context::default();
        for i in 0..60 {
            ctx.messages.push(Message::user_text(format!("msg{i}")));
        }
        let _ = run_agent(&backend, &model(), &mut ctx, &[], &sig(), None);

        // [summary user msg] + last 20 originals + final assistant.
        assert_eq!(ctx.messages.len(), 1 + 20 + 1);
        assert_eq!(
            ctx.messages[0],
            Message::user_text("[context summary]\nthe gist")
        );
        assert_eq!(ctx.messages[1], Message::user_text("msg40"));
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
