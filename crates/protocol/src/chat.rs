//! Wire-shaped chat message DTO.
//!
//! Mirrors the omenic agent-domain `llm::Message` shape exactly — same
//! serde field names, same declaration order (JSON `char` accounting in
//! `omenic-harness-compaction` depends on byte-identical encodings) — so a
//! JSON round-trip between the two is lossless. Harness crates speak this
//! instead of importing the agent domain (harness → agent is forbidden).
//!
//! Reference: `omenic crates/agent/adaptor/src/lib.rs` (`Message`/`Block`/
//! `Content`/`Role`); kept in sync as a DTO mirror, not a redesign.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Message role. Tool results ride inside user messages.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

/// Content block: text, image, tool invocation (assistant), or tool
/// result (user).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Text {
        text: String,
    },
    /// Image block for user turns: the payload ships as a `data:` URL to
    /// the model. Mirrors `llm::Block::Image` (field names and order).
    Image {
        /// MIME type, e.g. "image/png". Callers pass validated
        /// allow-listed values.
        media_type: String,
        /// Bare base64 payload, no "data:" prefix.
        data: String,
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

/// One conversation message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
}
