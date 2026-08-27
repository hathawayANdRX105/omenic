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

/// Call the chat-completions API with streaming and collect unified events.
///
/// Blocking (no async runtime in this crate); abort is polled between lines
/// via `signal`. The final event is always `Done` or `Error`.
///
// ponytail: collects instead of yielding deltas live — oi-core has no UI yet;
// switch to a callback/iterator when a TUI consumes it.
pub fn stream(
    model: &Model,
    context: &Context,
    tools: &[ToolDef],
    signal: &AtomicBool,
) -> Vec<StreamEvent> {
    if signal.load(Ordering::Relaxed) {
        return vec![StreamEvent::Done {
            stop_reason: StopReason::Aborted,
        }];
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
            return vec![StreamEvent::Error(format!("API {status}: {text}"))];
        }
        Err(e) => {
            if signal.load(Ordering::Relaxed) {
                return vec![StreamEvent::Done {
                    stop_reason: StopReason::Aborted,
                }];
            }
            return vec![StreamEvent::Error(e.to_string())];
        }
    };

    let reader = BufReader::new(response.into_reader());
    let mut parser = SseParser::new();
    let mut events = Vec::new();
    let mut stop_reason = StopReason::EndTurn;

    for line in reader.lines() {
        if signal.load(Ordering::Relaxed) {
            stop_reason = StopReason::Aborted;
            break;
        }
        let Ok(line) = line else {
            return vec![StreamEvent::Error("stream read failed".into())];
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
            events.push(StreamEvent::TextDelta(delta));
        }
        if let Some(reason) = out.stop_reason {
            stop_reason = reason;
        }
    }

    for tc in parser.flush() {
        events.push(StreamEvent::ToolCall(tc));
    }
    events.push(StreamEvent::Done { stop_reason });
    events
}
