//! Budget partitioning: what a model request spends on fixed overhead vs
//! the verbatim recent window.
//!
//! Reference: dsh `compaction-basic/src/region.ts` (`selectCompactableRange`
//! with a `retainTokens` verbatim tail). Ported to the orbit char-accounting
//! model: three regions — incompactable overhead (system prompt + tool
//! schemas, [`reserved_chars`]), the newest-message verbatim window
//! ([`recent_window_cut`]), and the compactable prefix between them.

use protocol::ToolSpec;

use crate::Message;
use crate::policy::message_chars;

/// Trigger budget: compact once the context (system prompt included)
/// reaches this many characters (~4 chars/token ⇒ roughly 30k tokens).
/// Ported from orbit `COMPACT_CHAR_BUDGET`.
pub const COMPACT_CHAR_BUDGET: usize = 120_000;
/// Characters of the newest messages kept verbatim during compaction.
/// Ported from orbit `KEEP_RECENT_CHARS`.
pub const KEEP_RECENT_CHARS: usize = 30_000;
/// Newest messages always kept verbatim, even when oversized on their own.
/// Ported from orbit `KEEP_RECENT_MIN`.
pub const KEEP_RECENT_MIN: usize = 2;

/// The three-region split of a request's character budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionBudget {
    /// Total request budget; compaction triggers at this size.
    pub total_chars: usize,
    /// Verbatim recent-window budget carved out of the total.
    pub keep_recent_chars: usize,
    /// Floor of newest messages kept whatever their size.
    pub keep_recent_min: usize,
}

impl Default for RegionBudget {
    fn default() -> Self {
        RegionBudget {
            total_chars: COMPACT_CHAR_BUDGET,
            keep_recent_chars: KEEP_RECENT_CHARS,
            keep_recent_min: KEEP_RECENT_MIN,
        }
    }
}

/// Per-request overhead compaction can never shrink: the system prompt and
/// the tool schemas ride every model call. Counted in serialized chars, the
/// same unit as [`message_chars`].
pub fn reserved_chars(system: Option<&str>, tools: &[ToolSpec]) -> usize {
    system.map_or(0, str::len)
        + tools
            .iter()
            .map(|t| serde_json::to_string(t).unwrap_or_default().len())
            .sum::<usize>()
}

/// First index to keep verbatim: walks newest → oldest spending `keep_chars`
/// characters, always keeping at least `keep_min` messages. `0` means the
/// whole context fits the window — nothing to compact.
///
/// Raw window only; pairing is [`crate::pairing`]’s job — see
/// [`crate::policy::select_compaction_cut`] for the composed cut.
pub fn recent_window_cut(messages: &[Message], keep_chars: usize, keep_min: usize) -> usize {
    let mut used = 0usize;
    let mut cut = messages.len();
    for (i, m) in messages.iter().enumerate().rev() {
        let size = message_chars(m);
        if used + size > keep_chars && messages.len() - i > keep_min {
            break;
        }
        used += size;
        cut = i;
    }
    cut
}
