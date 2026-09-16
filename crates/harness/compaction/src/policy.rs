//! The compaction policy trait and the orbit char-budget default.

use omenic_harness_core::chat::{Content, Message as ChatMessage};

use crate::pairing::advance_to_balanced;
use crate::region::{KEEP_RECENT_MIN, RegionBudget, recent_window_cut};
use crate::summarize::{NoopSummarizer, Summarizer, render_transcript};

/// Prefix of the injected summary message. Kept public because evidence
/// consumers (the context log replay, orbit EC-7) key on it.
pub const SUMMARY_PREFIX: &str = "[context summary]\n";

/// A host context-maintenance policy: rewrite a message list once it has
/// outgrown `budget` characters.
pub trait CompactionPolicy {
    /// Compacted replacement for `msgs` — or a verbatim clone when nothing
    /// may safely be dropped (in-fits, pairing, or summarizer failure).
    fn compact(&self, msgs: &[ChatMessage], budget: usize) -> Vec<ChatMessage>;
}

/// Estimated size of one message in characters: text measured directly,
/// block content through its JSON encoding (what the wire actually carries).
/// Ported verbatim from orbit `message_chars:255`.
///
/// ponytail: role/framing overhead uncounted — a rounding error next to
/// message bodies.
pub fn message_chars(m: &ChatMessage) -> usize {
    match &m.content {
        Content::Text(s) => s.len(),
        blocks => serde_json::to_string(blocks).unwrap_or_default().len(),
    }
}

/// First index to keep verbatim: the [`crate::region`] char-budget window
/// advanced to the nearest tool-pairing-balanced position
/// ([`crate::pairing`]). `0` means the whole context fits the recent window
/// — nothing to compact. Port of orbit `select_compaction_cut:285`, with the
/// orphan-skip loop upgraded to the full pairing invariant.
pub fn select_compaction_cut(messages: &[ChatMessage], budget: usize) -> usize {
    advance_to_balanced(
        messages,
        recent_window_cut(messages, budget, KEEP_RECENT_MIN),
    )
}

/// One maintenance pass. `system` contributes to both the trigger and the
/// kept-window guard (it rides every request and compaction cannot shrink
/// it); `budget` is the total request budget.
///
/// Returns the replacement list and — only when a replacement happened — the
/// summary message, so callers can log it before shipping (orbit EC-7
/// traceability). Every guard path returns a verbatim clone plus `None`
/// (orbit invariant 4: failures keep the original context untouched):
/// - whole context below budget, or nothing older than the recent window;
/// - the kept window alone already meets the budget — summarizing would
///   re-summarize the previous summary every turn;
/// - the summarizer failed or produced nothing.
pub fn compact_with(
    summarizer: &dyn Summarizer,
    system: Option<&str>,
    messages: &[ChatMessage],
    budget: usize,
    region: &RegionBudget,
) -> (Vec<ChatMessage>, Option<ChatMessage>) {
    let intact = || (messages.to_vec(), None);
    let system_chars = system.map_or(0, str::len);
    if system_chars + messages.iter().map(message_chars).sum::<usize>() < budget {
        return intact();
    }
    let cut = advance_to_balanced(
        messages,
        recent_window_cut(messages, region.keep_recent_chars, region.keep_recent_min),
    );
    if cut == 0 {
        return intact(); // nothing older than the recent window
    }
    // The newest `keep_recent_min` messages are kept whatever their size.
    // When they alone blow the budget, summarizing the prefix cannot get
    // under it — leave the context intact.
    let kept_chars = system_chars + messages[cut..].iter().map(message_chars).sum::<usize>();
    if kept_chars >= budget {
        return intact();
    }
    let Some(summary) = summarizer
        .summarize(&render_transcript(&messages[..cut]))
        .filter(|s| !s.is_empty())
    else {
        return intact();
    };
    let summary_msg = ChatMessage::user_text(format!("{SUMMARY_PREFIX}{summary}"));
    let mut out = vec![summary_msg.clone()];
    out.extend_from_slice(&messages[cut..]);
    (out, Some(summary_msg))
}

/// The orbit default policy: summarize the oldest prefix once the context
/// reaches the char budget, keeping the recent window verbatim. Reference:
/// orbit `compact_context:315` (logic ported here; orbit keeps the call
/// seam).
pub struct CharBudgetPolicy {
    summarizer: Box<dyn Summarizer + Send + Sync>,
    region: RegionBudget,
}

impl Default for CharBudgetPolicy {
    fn default() -> Self {
        Self::new(NoopSummarizer)
    }
}

impl CharBudgetPolicy {
    /// Policy with `summarizer` wired in and the default orbit regions.
    pub fn new(summarizer: impl Summarizer + Send + Sync + 'static) -> Self {
        Self::with_region(summarizer, RegionBudget::default())
    }

    pub fn with_region(
        summarizer: impl Summarizer + Send + Sync + 'static,
        region: RegionBudget,
    ) -> Self {
        CharBudgetPolicy {
            summarizer: Box::new(summarizer),
            region,
        }
    }

    /// Full pass with a system prompt in the accounting; see
    /// [`compact_with`].
    pub fn compact_context(
        &self,
        system: Option<&str>,
        messages: &[ChatMessage],
        budget: usize,
    ) -> (Vec<ChatMessage>, Option<ChatMessage>) {
        compact_with(
            self.summarizer.as_ref(),
            system,
            messages,
            budget,
            &self.region,
        )
    }

    /// Trigger budget of this policy (compaction fires once the context
    /// reaches this many characters). Read-only view for hosts that run the
    /// policy through their own seam: orbit's `LlmBackend` bridge streams the
    /// summary itself and only takes the budget + region from the resolved
    /// service, so a host-provided policy still decides when compaction fires.
    pub fn total_chars(&self) -> usize {
        self.region.total_chars
    }

    /// Verbatim recent-window partition of this policy; see [`RegionBudget`].
    pub fn region(&self) -> RegionBudget {
        self.region
    }
}

impl CompactionPolicy for CharBudgetPolicy {
    fn compact(&self, msgs: &[ChatMessage], budget: usize) -> Vec<ChatMessage> {
        self.compact_context(None, msgs, budget).0
    }
}
