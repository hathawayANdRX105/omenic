//! Char-budget context compaction policy (C4).
//!
//! Port of orbit's host maintenance policy (`select_compaction_cut` /
//! `compact_context`, omenic `crates/agent/orbit/src/lib.rs:285/:315`) out of
//! the agent domain into a harness service, hardened with the dsh compaction
//! invariants:
//! - [`pairing`] — a cut never splits a tool_use/tool_result pair
//!   (dsh `packages/compaction/compaction/src/tool-pairing.ts` balance fold);
//! - [`region`] — budget partition: fixed overhead (system prompt, tool
//!   schemas) vs the verbatim recent window
//!   (dsh `packages/compaction/compaction-basic/src/region.ts`);
//! - [`summarize`] — pluggable LLM summary hook, keep-original default
//!   (dsh `.../src/summarizer.ts`).
//!
//! Invariant 4 (orbit): on any summarizer failure the messages are kept
//! untouched. The orbit loop keeps only a call seam over this crate.

pub mod pairing;
pub mod plugin;
pub mod policy;
pub mod region;
pub mod summarize;

pub use omenic_harness_core::chat;
pub use omenic_harness_core::chat::Message;

pub use pairing::{advance_to_balanced, is_balanced_at};
pub use plugin::CompactionPlugin;
pub use policy::{
    CharBudgetPolicy, CompactionPolicy, SUMMARY_PREFIX, compact_with, message_chars,
    select_compaction_cut,
};
pub use region::{
    COMPACT_CHAR_BUDGET, KEEP_RECENT_CHARS, KEEP_RECENT_MIN, RegionBudget, recent_window_cut,
    reserved_chars,
};
pub use summarize::{NoopSummarizer, Summarizer, render_transcript};

/// Re-type a shape-mirrored wire message (e.g. `adaptor::Message`) into the
/// harness [`Message`] DTO via JSON. Lossless: [`chat`] mirrors the wire
/// shape field-for-field, in declaration order, so the [`message_chars`]
/// encoding is byte-identical to the source type's.
pub fn to_dto<M: serde::Serialize>(m: &M) -> Message {
    let value = serde_json::to_value(m).expect("wire messages are serializable");
    serde_json::from_value(value).expect("chat::Message mirrors the wire shape")
}

/// Inverse of [`to_dto`].
pub fn to_wire<W: serde::de::DeserializeOwned>(m: &Message) -> W {
    let value = serde_json::to_value(m).expect("DTO is serializable");
    serde_json::from_value(value).expect("chat::Message mirrors the wire shape")
}
