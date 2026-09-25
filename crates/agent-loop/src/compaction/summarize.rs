//! Summarization hook: the over-budget prefix becomes an LLM summary.
//!
//! Reference: dsh `compaction-basic/src/summarizer.ts` (the summary rides a
//! request built from the conversation's own shape) and orbit's inline
//! summary stream (`compact_context`, omenic
//! `crates/agent/orbit/src/lib.rs:366-394`). The LLM call itself is a
//! host-capability concern — the agent domain must not leak into the
//! harness — so this crate only abstracts it.

use protocol::chat::Content;

use super::Message;

/// Compresses the rendered transcript of the compactable prefix.
pub trait Summarizer {
    /// Return the summary text, or `None` on failure (backend error, abort,
    /// empty result) — a `None` makes compaction keep the original messages
    /// verbatim (orbit invariant 4).
    fn summarize(&self, transcript: &str) -> Option<String>;
}

/// Default: no summarizer wired — compaction keeps the original text.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopSummarizer;

impl Summarizer for NoopSummarizer {
    fn summarize(&self, _transcript: &str) -> Option<String> {
        None
    }
}

/// Render messages into the summarizer transcript: one `"<Role>: <content>"`
/// line each, block content as its JSON encoding. Ported verbatim from
/// orbit's inline rendering, so byte-for-byte the same text reaches the
/// summary backend.
pub fn render_transcript(messages: &[Message]) -> String {
    messages
        .iter()
        .map(|m| {
            format!(
                "{:?}: {}",
                m.role,
                match &m.content {
                    Content::Text(s) => s.clone(),
                    blocks => serde_json::to_string(blocks).unwrap_or_default(),
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
