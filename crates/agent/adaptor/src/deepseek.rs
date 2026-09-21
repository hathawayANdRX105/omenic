//! DeepSeek dialect wrapper for OpenAI-compatible LLM streaming.
//!
//! DeepSeek's chat-completions API is OpenAI-compatible except:
//! - `max_tokens` is required — requests without it fail with HTTP 400.
//! - `deepseek-reasoner` streams a `reasoning_content` delta that the shared
//!   OpenAI SSE parser does not model.
//!
//! This wrapper fixes the first (defaulting `max_tokens`) and delegates the
//! rest to the OpenAI adapter. Follow-up (not in this batch): surface
//! `reasoning_content` as a `StreamEvent::ReasoningDelta` variant — that needs
//! new arms in `orbit::lib.rs`, `orbit::fallback.rs`, and
//! `web/state/memory_link.rs`, which currently match `StreamEvent`
//! exhaustively.

use std::sync::atomic::AtomicBool;

use crate::{Context, Model, StreamEvent, ToolDef};

/// DeepSeek's documented default output cap when the caller sets none.
pub const DEEPSEEK_DEFAULT_MAX_TOKENS: u32 = 8192;

/// Whether `model` should take the DeepSeek dialect.
///
/// Heuristic (ponytail: no new config field — `Model` is constructed in 10+
/// places): the model id starts with `deepseek`, or the base URL mentions
/// `deepseek`. Known limitation: a self-hosted DeepSeek-compatible endpoint
/// whose URL does not contain "deepseek" and whose model is not named
/// `deepseek-*` keeps the OpenAI dialect.
pub fn is_deepseek_model(model: &Model) -> bool {
    model.model.starts_with("deepseek")
        || model
            .base_url
            .as_ref()
            .is_some_and(|url| url.contains("deepseek"))
}

/// `max_tokens` to send: the caller's value, or the DeepSeek default.
pub fn effective_max_tokens(model: &Model) -> u32 {
    model.max_tokens.unwrap_or(DEEPSEEK_DEFAULT_MAX_TOKENS)
}

/// DeepSeek dialect entry point: defaults `max_tokens`, then delegates.
pub fn stream_cb(
    model: &Model,
    context: &Context,
    tools: &[ToolDef],
    signal: &AtomicBool,
    emit: &mut dyn FnMut(&StreamEvent),
) {
    let mut effective = model.clone();
    effective.max_tokens = Some(effective_max_tokens(model));
    super::openai::stream_cb(&effective, context, tools, signal, emit);
}
