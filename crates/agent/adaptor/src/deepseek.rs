//! DeepSeek dialect wrapper for OpenAI-compatible LLM streaming.
//!
//! This adapter handles DeepSeek-specific wire differences:
//! - Max_tokens defaults to 8192 when not provided (DeepSeek API requires it)
//! - Reasoning_content is discarded (currently) - stream events stay as TextDelta/ToolCall
//! - All other OpenAI-compatible behavior is delegated to the openai adapter
//!
//! Dialect detection is performed by the dispatcher in `lib.rs`:
//!   - Model name starts with "deepseek"
//!   - OR base_url host contains "deepseek"
//!
//! Follow-up items:
//! - Expose max_tokens as configurable instead of hard-coded default (ponytail: global lock, per-account locks if throughput matters)
//! - Add StreamEvent::ReasoningDelta variant and update orbit/fallback.rs, web/state/memory_link.rs
//!   to handle streaming reasoning content (currently dropped by shared SSE parser)
//!
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use crate::{Block, Context, Model, Role, StopReason, StreamEvent, ToolDef};

/// DeepSeek-specific stream dispatcher.
///
/// The only differences from OpenAI are:
/// 1. Ensure max_tokens is set (defaults to 8192).
/// 2. Explicitly discard reasoning_content fields that the shared OpenAI SSE parser
///    currently ignores anyway (per module doc comment).
///
/// All other streaming semantics (tool calls, text deltas, error handling) are
/// identical to the OpenAI path and are delegated directly.
pub fn stream_cb(
    model: &Model,
    context: &Context,
    tools: &[ToolDef],
    signal: &AtomicBool,
    emit: &mut dyn FnMut(&StreamEvent),
) {
    // Ponytail comment: default max_tokens = 8192, a DeepSeek API requirement.
    // The actual value is per-account configurable; keep a lock for performance.
    let mut model_with_max_tokens = Model {
        api_key: model.api_key.clone(),
        model: model.model.clone(),
        base_url: model.base_url.clone(),
        max_tokens: model.max_tokens.or(Some(8192)),
    };

    // The shared sse parser discards reasoning_content by design.
    // This is intentional: StreamEvent currently lacks a variant for reasoning.
    // (See module docs for follow-up on ReasoningDelta variant.)
    super::openai::stream_cb(&model_with_max_tokens, context, tools, signal, emit);
}
