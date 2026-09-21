//! LLM stream adaptor: OpenAI-compatible SSE → unified events.
//!
//! Types live here as the crate root. SSE parsing in `sse`, HTTP call in `openai`,
//! DeepSeek dialect wrapper in `deepseek` (max_tokens default + reasoning_content
//! drop — see its module docs).

pub mod deepseek;
pub mod openai;
pub mod sse;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::AtomicBool;

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
/// Serde-wise this is the payload of `orbit::AgentEvent::ToolCall`; the
/// field names are the cross-crate event contract (R2 3.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// Dispatcher: routes to DeepSeek or OpenAI dialect based on model/base_url.
///
/// - DeepSeek if `model.model` starts with "deepseek" or `base_url` contains "deepseek"
/// - Otherwise delegates to `openai::stream_cb` (zero behavior change for OpenAI)
pub fn stream_cb(
    model: &Model,
    context: &Context,
    tools: &[ToolDef],
    signal: &AtomicBool,
    emit: &mut dyn FnMut(&StreamEvent),
) {
    if model.model.starts_with("deepseek")
        || model
            .base_url
            .as_ref()
            .map_or(false, |url| url.contains("deepseek"))
    {
        deepseek::stream_cb(model, context, tools, signal, emit);
    } else {
        openai::stream_cb(model, context, tools, signal, emit);
    }
}
