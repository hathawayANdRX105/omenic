//! OpenAI-compatible chat-completions streaming.

use std::io::BufRead;
use std::io::BufReader;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};

use crate::sse::SseParser;
use crate::{Block, Content, Context, Message, Model, Role, StopReason, StreamEvent, ToolDef};

pub(crate) fn context_to_openai_messages(context: &Context) -> Vec<Value> {
    let mut messages = Vec::with_capacity(context.messages.len() + 1);
    if let Some(system) = &context.system_prompt {
        messages.push(json!({ "role": "system", "content": system }));
    }
    for m in &context.messages {
        let content = match &m.content {
            Content::Text(s) => json!(s),
            Content::Blocks(blocks) => Value::Array(
                blocks
                    .iter()
                    .map(|b| match b {
                        Block::Text { text } => json!({ "type": "text", "text": text }),
                        Block::ToolUse { id, name, input } => json!({
                            "type": "tool_use", "id": id, "name": name, "input": input,
                        }),
                        Block::ToolResult {
                            tool_use_id,
                            content,
                        } => json!({
                            "type": "tool_result", "tool_use_id": tool_use_id, "content": content,
                        }),
                    })
                    .collect(),
            ),
        };
        let role = match m.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        messages.push(json!({ "role": role, "content": content }));
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

    let response = match ureq::post(&url)
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
