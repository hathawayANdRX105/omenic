//! OpenAI-compatible chat-completions streaming.

use std::io::BufRead;
use std::io::BufReader;
use std::sync::atomic::{AtomicBool, Ordering};

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
                let mut flush_assistant =
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

// ===== streaming call =====

/// Call the chat-completions API with streaming, invoking `emit` for each event.
///
/// Blocking (no async runtime); abort is polled between lines via `signal`.
/// The final event is always `Done` or `Error`. Returns immediately after
/// the terminal event is emitted.
pub fn stream_cb(
    model: &Model,
    context: &Context,
    tools: &[ToolDef],
    signal: &AtomicBool,
    emit: &mut dyn FnMut(&StreamEvent),
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
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout_read(std::time::Duration::from_secs(90))
        .build();
    let response = match agent
        .post(&url)
        .set("Authorization", &format!("Bearer {}", model.api_key))
        .send_json(request_body(model, context, tools))
    {
        Ok(r) => r,
        Err(ureq::Error::Status(status, resp)) => {
            let text = resp
                .into_string()
                .unwrap_or_else(|_| "unknown error".into());
            emit(&StreamEvent::Error(format!("API {status}: {text}")));
            return;
        }
        Err(e) => {
            if signal.load(Ordering::Relaxed) {
                emit(&StreamEvent::Done {
                    stop_reason: StopReason::Aborted,
                });
            } else {
                emit(&StreamEvent::Error(e.to_string()));
            }
            return;
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
            emit(&StreamEvent::Error("stream read failed".into()));
            return;
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
