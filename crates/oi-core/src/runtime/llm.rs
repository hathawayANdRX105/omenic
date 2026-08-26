//! Unified LLM stream: OpenAI-compatible SSE → four event kinds.
//!
//! Rust port of pi-from-scratch `src/llm.ts`. Context is plain JSON
//! (serde) so it can be persisted and restored losslessly.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Model configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct Model {
    pub api_key: String,
    pub model: String,
    /// Defaults to `https://api.openai.com/v1`.
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

/// Content block: text, tool invocation (assistant), or tool result (user).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

/// Message content: plain string or block list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Blocks(Vec<Block>),
}

/// Message role. tool_results ride inside user messages.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

/// One conversation message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub role: Role,
    pub content: Content,
}

impl Message {
    pub fn user_text(text: impl Into<String>) -> Self {
        Message {
            role: Role::User,
            content: Content::Text(text.into()),
        }
    }

    pub fn assistant_text(text: impl Into<String>) -> Self {
        Message {
            role: Role::Assistant,
            content: Content::Text(text.into()),
        }
    }

    pub fn assistant(text: String, tool_calls: &[ToolCallSpec]) -> Self {
        let mut blocks = vec![];
        if !text.is_empty() {
            blocks.push(Block::Text { text });
        }
        for tc in tool_calls {
            blocks.push(Block::ToolUse {
                id: tc.id.clone(),
                name: tc.name.clone(),
                input: tc.args.clone(),
            });
        }
        Message {
            role: Role::Assistant,
            content: Content::Blocks(blocks),
        }
    }

    pub fn tool_results(results: &[(String, String)]) -> Self {
        Message {
            role: Role::User,
            content: Content::Blocks(
                results
                    .iter()
                    .map(|(id, content)| Block::ToolResult {
                        tool_use_id: id.clone(),
                        content: content.clone(),
                    })
                    .collect(),
            ),
        }
    }
}

/// Conversation context: system prompt + messages. Pure JSON, persistable.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Context {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub messages: Vec<Message>,
}

/// Why the model stopped generating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Aborted,
}

/// A completed tool call extracted from the stream.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallSpec {
    pub id: String,
    pub name: String,
    pub args: Value,
}

/// Tool definition sent to the API.
#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    /// JSON Schema for parameters.
    pub parameters: Value,
}

/// Unified stream events (the llm module's entire output surface).
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    TextDelta(String),
    ToolCall(ToolCallSpec),
    Done { stop_reason: StopReason },
    Error(String),
}

// ===== SSE parsing =====

#[derive(Debug, Default)]
struct ToolCallBuf {
    id: String,
    name: String,
    args_buf: String,
}

/// Incremental SSE chunk parser. Accumulates partial tool_calls by index;
/// pure logic so chunk-boundary behavior is unit-testable offline.
#[derive(Debug, Default)]
pub struct SseParser {
    tool_calls: BTreeMap<u64, ToolCallBuf>,
}

/// What one SSE `data:` line contributed.
#[derive(Debug, Default, PartialEq)]
pub struct SseLineOut {
    pub text_delta: Option<String>,
    pub stop_reason: Option<StopReason>,
}

impl SseParser {
    pub fn new() -> Self {
        SseParser::default()
    }

    /// Parse one `data:` payload (without the `data: ` prefix).
    /// Malformed JSON lines are ignored, matching llm.ts.
    pub fn handle_data(&mut self, data: &str) -> SseLineOut {
        let Ok(chunk) = serde_json::from_str::<Value>(data) else {
            return SseLineOut::default();
        };
        let Some(choice) = chunk["choices"].get(0) else {
            return SseLineOut::default();
        };

        let mut out = SseLineOut::default();

        if let Some(text) = choice["delta"]["content"].as_str() {
            if !text.is_empty() {
                out.text_delta = Some(text.to_string());
            }
        }

        // tool_call deltas: accumulate name + partial-JSON arguments by index.
        if let Some(tcs) = choice["delta"]["tool_calls"].as_array() {
            for tc in tcs {
                let idx = tc["index"].as_u64().unwrap_or(0);
                let entry = self.tool_calls.entry(idx).or_insert_with(|| ToolCallBuf {
                    id: tc["id"]
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("call_{idx}")),
                    ..ToolCallBuf::default()
                });
                if let Some(id) = tc["id"].as_str() {
                    entry.id = id.to_string();
                }
                if let Some(name) = tc["function"]["name"].as_str() {
                    entry.name = name.to_string();
                }
                if let Some(args) = tc["function"]["arguments"].as_str() {
                    entry.args_buf.push_str(args);
                }
            }
        }

        // finish_reason map: tool_calls→tool_use, length→max_tokens, stop→implicit end_turn.
        out.stop_reason = match choice["finish_reason"].as_str() {
            Some("tool_calls") => Some(StopReason::ToolUse),
            Some("length") => Some(StopReason::MaxTokens),
            _ => None,
        };

        out
    }

    /// Flush accumulated tool_calls in index order; args parsed or `{}` on bad JSON.
    pub fn flush(&self) -> Vec<ToolCallSpec> {
        self.tool_calls
            .values()
            .map(|tc| {
                let args = if tc.args_buf.is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(&tc.args_buf).unwrap_or_else(|_| json!({}))
                };
                ToolCallSpec {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    args,
                }
            })
            .collect()
    }
}

// ===== request building =====

fn context_to_openai_messages(context: &Context) -> Vec<Value> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn chunk(delta: Value, finish: Option<&str>) -> String {
        let mut c = json!({ "choices": [{ "delta": delta }] });
        if let Some(f) = finish {
            c["choices"][0]["finish_reason"] = json!(f);
        }
        c.to_string()
    }

    #[test]
    fn text_delta_passes_through() {
        let mut p = SseParser::new();
        assert_eq!(
            p.handle_data(&chunk(json!({"content": "he"}), None))
                .text_delta,
            Some("he".into())
        );
        assert_eq!(
            p.handle_data(&chunk(json!({"content": "llo"}), None))
                .text_delta,
            Some("llo".into())
        );
        assert!(p.flush().is_empty());
    }

    #[test]
    fn tool_call_args_reassembled_across_chunks() {
        let mut p = SseParser::new();
        // First fragment carries id+name, later fragments append argument JSON.
        let first = json!({"tool_calls": [{"index": 0, "id": "call_1", "function": {"name": "read_file", "arguments": "{\"pa"}}]});
        let second =
            json!({"tool_calls": [{"index": 0, "function": {"arguments": "th\": \"/tmp/x\"}"}}]});
        assert_eq!(p.handle_data(&chunk(first, None)), SseLineOut::default());
        assert_eq!(p.handle_data(&chunk(second, None)), SseLineOut::default());

        let calls = p.flush();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].args, json!({"path": "/tmp/x"}));
    }

    #[test]
    fn multiple_tool_calls_keep_index_order_and_defaults() {
        let mut p = SseParser::new();
        // index 1 arrives before index 0; index 2 has no explicit id.
        let a = json!({"tool_calls": [{"index": 1, "id": "b", "function": {"name": "edit", "arguments": "{}"}}]});
        let b = json!({"tool_calls": [{"index": 0, "id": "a", "function": {"name": "run_bash"}}]});
        let c = json!({"tool_calls": [{"index": 2, "function": {"name": "write_file", "arguments": "not-json"}}]});
        p.handle_data(&chunk(a, None));
        p.handle_data(&chunk(b, None));
        p.handle_data(&chunk(c, None));

        let calls = p.flush();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].id, "a");
        assert_eq!(calls[1].id, "b");
        assert_eq!(
            calls[2].args,
            json!({}),
            "unparseable args fall back to empty object"
        );
    }

    #[test]
    fn finish_reason_mapping() {
        let mut p = SseParser::new();
        assert_eq!(
            p.handle_data(&chunk(json!({}), Some("tool_calls")))
                .stop_reason,
            Some(StopReason::ToolUse)
        );
        assert_eq!(
            p.handle_data(&chunk(json!({}), Some("length"))).stop_reason,
            Some(StopReason::MaxTokens)
        );
        // "stop" and missing both leave EndTurn as the stream-level default.
        assert_eq!(
            p.handle_data(&chunk(json!({}), Some("stop"))),
            SseLineOut::default()
        );
        assert_eq!(p.handle_data("not json"), SseLineOut::default());
    }

    #[test]
    fn context_conversion_round_trip_shape() {
        let ctx = Context {
            system_prompt: Some("be brief".into()),
            messages: vec![
                Message::user_text("hi"),
                Message::assistant(
                    String::new(),
                    &[ToolCallSpec {
                        id: "t1".into(),
                        name: "read_file".into(),
                        args: json!({"path": "/x"}),
                    }],
                ),
                Message::tool_results(&[("t1".into(), "contents".into())]),
            ],
        };
        let msgs = context_to_openai_messages(&ctx);
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[2]["role"], "assistant");
        assert_eq!(msgs[2]["content"][0]["type"], "tool_use");
        assert_eq!(msgs[3]["role"], "user");
        assert_eq!(msgs[3]["content"][0]["type"], "tool_result");

        // Context serializes to plain JSON and restores losslessly.
        let s = serde_json::to_string(&ctx).unwrap();
        assert_eq!(serde_json::from_str::<Context>(&s).unwrap(), ctx);
    }

    #[test]
    fn aborted_before_request_yields_done() {
        let model = Model {
            api_key: "k".into(),
            model: "gpt-test".into(),
            base_url: None,
            max_tokens: None,
        };
        let signal = AtomicBool::new(true);
        let events = stream(&model, &Context::default(), &[], &signal);
        assert_eq!(
            events,
            vec![StreamEvent::Done {
                stop_reason: StopReason::Aborted
            }]
        );
    }
}
