//! OpenAI-compatible chat-completions streaming.

use std::io::BufRead;
use std::io::BufReader;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

use crate::sse::SseParser;
use crate::{Block, Content, Context, Model, Role, StopReason, StreamEvent, ToolDef};

pub(crate) fn context_to_openai_messages(context: &Context) -> Vec<Value> {
    let mut messages = Vec::with_capacity(context.messages.len() + 1);
    if let Some(system) = &context.system_prompt {
        messages.push(json!({ "role": "system", "content": system }));
    }
    for m in &context.messages {
        let role = match m.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        match &m.content {
            Content::Text(s) => {
                messages.push(json!({ "role": role, "content": s }));
            }
            Content::Blocks(blocks) => {
                // OpenAI 规范协议(不是 Anthropic blocks 格式):
                // - tool_use    → 并入 assistant 消息的 `tool_calls` 数组
                // - tool_result → 独立的 role:"tool" 消息,必须排在 assistant tool_calls 之后
                // 做法:先把 ToolUse/Text 收集为 pending assistant,碰到 ToolResult(或 blocks 末尾)就落盘。
                let mut pending_text = String::new();
                let mut pending_calls: Vec<Value> = Vec::new();
                let flush_assistant =
                    |text: &mut String, calls: &mut Vec<Value>, messages: &mut Vec<Value>| {
                        if text.is_empty() && calls.is_empty() {
                            return;
                        }
                        let mut msg = json!({ "role": role });
                        msg["content"] = if text.is_empty() {
                            Value::Null
                        } else {
                            json!(std::mem::take(text))
                        };
                        if !calls.is_empty() {
                            msg["tool_calls"] = Value::Array(std::mem::take(calls));
                        }
                        messages.push(msg);
                    };
                for b in blocks {
                    match b {
                        Block::Text { text } => {
                            if !pending_text.is_empty() {
                                pending_text.push('\n');
                            }
                            pending_text.push_str(text);
                        }
                        Block::ToolUse { id, name, input } => {
                            pending_calls.push(json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": serde_json::to_string(input).unwrap_or_default(),
                                }
                            }));
                        }
                        Block::ToolResult {
                            tool_use_id,
                            content,
                        } => {
                            flush_assistant(&mut pending_text, &mut pending_calls, &mut messages);
                            messages.push(json!({
                                "role": "tool",
                                "tool_call_id": tool_use_id,
                                "content": content,
                            }));
                        }
                    }
                }
                flush_assistant(&mut pending_text, &mut pending_calls, &mut messages);
            }
        }
    }
    messages
}

fn request_body(model: &Model, context: &Context, tools: &[ToolDef]) -> Value {
    let mut body = json!({
        "model": model.model,
        "stream": true,
        "messages": context_to_openai_messages(context),
    });
    if let Some(max) = model.max_tokens {
        body["max_tokens"] = json!(max);
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(
            tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters,
                        },
                    })
                })
                .collect(),
        );
    }
    body
}

// ===== retry policy =====

/// How often and how long to retry a failed LLM call (Claude Code-style
/// resilience: 429/5xx and broken connections retry, client errors and
/// mid-stream failures after emitted deltas do not).
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Total tries per LLM call, initial attempt included.
    pub max_attempts: u32,
    /// First backoff; doubles each retry.
    pub base_delay_ms: u64,
    /// Backoff ceiling.
    pub max_delay_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy {
            max_attempts: 4,
            base_delay_ms: 500,
            max_delay_ms: 10_000,
        }
    }
}

/// 429 and 5xx are the provider's fault; retry. 4xx (bad key, malformed
/// request) will fail identically every time; do not.
pub fn is_retryable_status(status: u16) -> bool {
    status == 429 || (500..600).contains(&status)
}

/// Dead connections are worth another try; anything else (TLS, proxy
/// config) fails deterministically.
pub fn is_retryable_transport(kind: ureq::ErrorKind) -> bool {
    matches!(
        kind,
        ureq::ErrorKind::ConnectionFailed | ureq::ErrorKind::Dns | ureq::ErrorKind::Io
    )
}

/// A failed round-trip: what happened, and whether a retry could fix it.
struct RoundFailure {
    message: String,
    retryable: bool,
    /// `Retry-After` seconds from a 429 response, when present.
    retry_after_ms: Option<u64>,
}

/// Sleep for `delay`, returning early (true) if the abort signal fires.
fn sleep_abortable(delay: Duration, signal: &AtomicBool) -> bool {
    let step = Duration::from_millis(25);
    let mut left = delay;
    while left > Duration::ZERO {
        let chunk = left.min(step);
        std::thread::sleep(chunk);
        if signal.load(Ordering::Relaxed) {
            return true;
        }
        left -= chunk;
    }
    signal.load(Ordering::Relaxed)
}

// ===== streaming call =====

/// Call the chat-completions API with streaming, invoking `emit` for each event.
///
/// Blocking (no async runtime); abort is polled between lines via `signal`.
/// The final event is always `Done` or `Error`. Returns immediately after
/// the terminal event is emitted.
///
/// Transient failures (429/5xx, dead connections, a stream that dies before
/// producing any text) are retried with exponential backoff per
/// [`RetryPolicy::default`]; a `Retry-After` header on 429 overrides the
/// backoff. Once a text delta has been emitted the round is never replayed.
pub fn stream_cb(
    model: &Model,
    context: &Context,
    tools: &[ToolDef],
    signal: &AtomicBool,
    emit: &mut dyn FnMut(&StreamEvent),
) {
    stream_cb_with_policy(model, context, tools, signal, emit, RetryPolicy::default());
}

/// [`stream_cb`] with an explicit retry policy (tests use tiny delays).
pub fn stream_cb_with_policy(
    model: &Model,
    context: &Context,
    tools: &[ToolDef],
    signal: &AtomicBool,
    emit: &mut dyn FnMut(&StreamEvent),
    policy: RetryPolicy,
) {
    if signal.load(Ordering::Relaxed) {
        emit(&StreamEvent::Done {
            stop_reason: StopReason::Aborted,
        });
        return;
    }

    let base = model
        .base_url
        .clone()
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));

    // Per-socket-read timeout so a stalled gateway fails instead of hanging the
    // agent thread forever. 90s covers slow long-thinking models between deltas.
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(90))
        .build();

    let mut attempt = 0u32;
    loop {
        attempt += 1;
        // Any text already handed to the consumer: after the first delta a
        // failed round must surface as an error, not a replayed call.
        let mut emitted_text = false;
        let failure = stream_round_trip(
            &url,
            &agent,
            model,
            context,
            tools,
            signal,
            emit,
            &mut emitted_text,
        );
        match failure {
            None => return, // terminal event already emitted
            Some(f) => {
                let can_retry = f.retryable && !emitted_text && attempt < policy.max_attempts;
                if !can_retry {
                    emit(&StreamEvent::Error(f.message));
                    return;
                }
                let delay = backoff_delay(attempt, f.retry_after_ms, &policy);
                if sleep_abortable(delay, signal) {
                    emit(&StreamEvent::Done {
                        stop_reason: StopReason::Aborted,
                    });
                    return;
                }
            }
        }
    }
}

/// Exponential backoff for attempt `attempt` (1-based), capped, with a
/// `Retry-After` override when the server sent one.
pub fn backoff_delay(attempt: u32, retry_after_ms: Option<u64>, policy: &RetryPolicy) -> Duration {
    let ms = match retry_after_ms {
        Some(ra) => ra.min(policy.max_delay_ms),
        None => policy
            .base_delay_ms
            .saturating_mul(1u64 << (attempt - 1).min(16))
            .min(policy.max_delay_ms),
    };
    Duration::from_millis(ms)
}

/// One HTTP round-trip: send the request and stream events until the turn
/// ends. Returns `None` when a terminal event was emitted (success path,
/// abort, or an error that must not be retried), or `Some(failure)` when
/// the caller may retry.
#[allow(clippy::too_many_arguments)]
fn stream_round_trip(
    url: &str,
    agent: &ureq::Agent,
    model: &Model,
    context: &Context,
    tools: &[ToolDef],
    signal: &AtomicBool,
    emit: &mut dyn FnMut(&StreamEvent),
    emitted_text: &mut bool,
) -> Option<RoundFailure> {
    let response = match agent
        .post(url)
        .set("Authorization", &format!("Bearer {}", model.api_key))
        .send_json(request_body(model, context, tools))
    {
        Ok(r) => r,
        Err(ureq::Error::Status(status, resp)) => {
            let retry_after_ms = resp
                .header("retry-after")
                .and_then(|v| v.trim().parse::<u64>().ok())
                .map(|secs| secs.saturating_mul(1000));
            let text = resp
                .into_string()
                .unwrap_or_else(|_| "unknown error".into());
            return Some(RoundFailure {
                message: format!("API {status}: {text}"),
                retryable: is_retryable_status(status),
                retry_after_ms,
            });
        }
        Err(e) => {
            if signal.load(Ordering::Relaxed) {
                emit(&StreamEvent::Done {
                    stop_reason: StopReason::Aborted,
                });
                return None;
            }
            let retryable =
                matches!(&e, ureq::Error::Transport(t) if is_retryable_transport(t.kind()));
            return Some(RoundFailure {
                message: e.to_string(),
                retryable,
                retry_after_ms: None,
            });
        }
    };

    let reader = BufReader::new(response.into_reader());
    let mut parser = SseParser::new();
    let mut stop_reason = StopReason::EndTurn;

    for line in reader.lines() {
        if signal.load(Ordering::Relaxed) {
            stop_reason = StopReason::Aborted;
            break;
        }
        let Ok(line) = line else {
            // The socket died mid-stream. Retrying is only safe when the
            // consumer has seen nothing — otherwise a replay would
            // duplicate emitted deltas.
            if *emitted_text {
                emit(&StreamEvent::Error("stream read failed".into()));
                return None;
            }
            return Some(RoundFailure {
                message: "stream read failed".into(),
                retryable: true,
                retry_after_ms: None,
            });
        };
        let line = line.trim();
        if !line.starts_with("data: ") {
            continue;
        }
        let data = &line["data: ".len()..];
        if data == "[DONE]" {
            continue;
        }
        let out = parser.handle_data(data);
        if let Some(delta) = out.text_delta {
            *emitted_text = true;
            emit(&StreamEvent::TextDelta(delta));
        }
        if let Some(reason) = out.stop_reason {
            stop_reason = reason;
        }
    }

    for tc in parser.flush() {
        emit(&StreamEvent::ToolCall(tc));
    }
    emit(&StreamEvent::Done { stop_reason });
    None
}

/// Collecting wrapper: calls `stream_cb` and gathers all events into a Vec.
/// Use `stream_cb` directly when you need live per-event processing.
pub fn stream(
    model: &Model,
    context: &Context,
    tools: &[ToolDef],
    signal: &AtomicBool,
) -> Vec<StreamEvent> {
    let mut events = Vec::new();
    stream_cb(model, context, tools, signal, &mut |ev| {
        events.push(ev.clone());
    });
    events
}
