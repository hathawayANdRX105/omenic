//! Injection pipeline: turn a recalled memory set into a `<system-reminder>`
//! payload, and gate re-injection so the same content never spams the
//! transcript.
//!
//! The four gate layers port jcode `memory/pending.rs`, adapted from its
//! session-keyed statics to a caller-owned [`InjectionBuffer`] whose clock is
//! injected as plain unix seconds (no wall-clock reads, deterministic tests):
//!
//! 1. freshness — a pending set dies [`FRESH_SECS`] after it was written;
//! 2. fully known — a set identical to the last injected one never re-runs
//!    (windowless by design; callers recreate the buffer to reset, mirroring
//!    jcode's session-scoped state);
//! 3. signature — the same normalized content signature is suppressed for
//!    [`SIGNATURE_SECS`] after the last injection;
//! 4. overlap — a set covering ≥80% of the last injected set (jcode's
//!    baseline: the larger of the two) is suppressed for [`OVERLAP_SECS`].
//!
//! Gates (b)–(d) only park the pending set, they do not drop it: once the
//! suppression windows pass, a later [`InjectionBuffer::take_ready`] can
//! still inject it. Only freshness and actual injection clear the buffer.

use std::collections::{HashSet, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};

use crate::{Category, MemoryEntry};

/// A pending set stays injectable this long after `set_memories`
/// (jcode: `PendingMemory::is_fresh`).
const FRESH_SECS: u64 = 120;
/// The same content signature is suppressed this long after the last
/// injection (jcode: `MEMORY_REPEAT_SUPPRESSION_SECS`).
const SIGNATURE_SECS: u64 = 90;
/// A ≥80%-overlapping set is suppressed this long after the last injection
/// (jcode: `MEMORY_SET_REPEAT_SUPPRESSION_SECS`).
const OVERLAP_SECS: u64 = 180;
/// Overlap fraction above which a set counts as "already surfaced", in
/// percent of the larger set (jcode: `MEMORY_SET_OVERLAP_SUPPRESSION_RATIO`
/// = 0.8 of max size). Integer math so the threshold never trips on float
/// rounding.
const OVERLAP_MIN_PCT: u64 = 80;

/// Render order and headings for [`format_injection`] (jcode prompt order).
const SECTIONS: &[(Category, &str)] = &[
    (Category::Correction, "Corrections"),
    (Category::Preference, "Preferences"),
    (Category::Fact, "Facts"),
    (Category::Entity, "Entities"),
    (Category::Custom, "Notes"),
];

/// Render `memories` as a `<system-reminder>` payload, grouped by category
/// in fixed order (Correction → Preference → Fact → Entity → Custom), every
/// entry whitespace-normalized and numbered per section. Sections without
/// entries are omitted; when nothing is renderable at all (no entries, or
/// only blank text) the result is an empty string — callers must not inject
/// an empty payload.
pub fn format_injection(memories: &[MemoryEntry]) -> String {
    let mut body = String::new();
    for (category, title) in SECTIONS {
        let items: Vec<String> = memories
            .iter()
            .filter(|e| e.category == *category)
            .map(|e| normalize_ws(&e.text))
            .filter(|text| !text.is_empty())
            .collect();
        if items.is_empty() {
            continue;
        }
        if body.is_empty() {
            body.push_str("<system-reminder>\n# Memory\n");
        }
        body.push_str("## ");
        body.push_str(title);
        body.push('\n');
        for (i, item) in items.iter().enumerate() {
            body.push_str(&format!("{}. {}\n", i + 1, item));
        }
    }
    if body.is_empty() {
        return String::new();
    }
    body.push_str("</system-reminder>");
    body
}

/// Caller-owned injection gate over one pending memory set. Not a static —
/// the orchestrator holds it per session/loop, so state dies with the handle
/// and tests never need a global reset.
///
/// All time flows in as unix seconds: `set_memories` stamps the write time,
/// `take_ready` gets the current time.
#[derive(Debug, Clone, Default)]
pub struct InjectionBuffer {
    /// Waiting to be injected; replaced wholesale by `set_memories`.
    pending: Vec<MemoryEntry>,
    /// When `pending` was written (unix seconds).
    written_at: Option<u64>,
    /// Normalized texts of the last injected set — gates (b) and (d). Text,
    /// not ids: freshly built entries may all carry id 0.
    last_injected: Option<HashSet<String>>,
    /// Signature of the last injected set — gate (c).
    last_signature: Option<u64>,
    /// When the last injection happened (unix seconds).
    last_injected_at: Option<u64>,
}

impl InjectionBuffer {
    /// Empty buffer, nothing injected yet.
    pub fn new() -> InjectionBuffer {
        InjectionBuffer::default()
    }

    /// Replace the pending set; `now` (unix seconds) restarts its freshness
    /// clock.
    pub fn set_memories(&mut self, memories: Vec<MemoryEntry>, now: u64) {
        self.pending = memories;
        self.written_at = Some(now);
    }

    /// Whether anything is waiting to be injected.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Take the pending set if it survives all four gates, returning the
    /// formatted `<system-reminder>` payload. `now` is unix seconds.
    ///
    /// On pass the injected set, its signature and `now` are recorded and the
    /// pending set cleared. On a freshness miss the pending set is dropped
    /// for good; gates (b)–(d) leave it parked for a later retry.
    pub fn take_ready(&mut self, now: u64) -> Option<String> {
        if self.pending.is_empty() {
            return None;
        }
        let written_at = self.written_at?;

        // (a) freshness: a stale payload is dropped, never injected late.
        if now.saturating_sub(written_at) >= FRESH_SECS {
            self.clear_pending();
            return None;
        }

        let known = identity_set(&self.pending);
        if known.is_empty() {
            // Blank texts only — nothing renderable, drop instead of looping.
            self.clear_pending();
            return None;
        }
        let signature = set_signature(&known);

        // (b) fully known: identical to the last injected set.
        if self.last_injected.as_ref() == Some(&known) {
            return None;
        }

        // (c) signature: same content shortly after the last injection is a
        // near-immediate duplicate. Narrower than (b) by construction (equal
        // sets always hash equal); kept as an independent layer mirroring
        // jcode so relaxing one gate cannot silently drop the other.
        if self.last_signature == Some(signature)
            && self
                .last_injected_at
                .is_some_and(|at| now.saturating_sub(at) < SIGNATURE_SECS)
        {
            return None;
        }

        // (d) overlap: mostly the same set shortly after the last injection
        // is a reshuffle, not news.
        if self
            .last_injected_at
            .is_some_and(|at| now.saturating_sub(at) < OVERLAP_SECS)
            && self
                .last_injected
                .as_ref()
                .is_some_and(|last| overlaps_enough(last, &known))
        {
            return None;
        }

        let payload = format_injection(&self.pending);
        if payload.is_empty() {
            self.clear_pending();
            return None;
        }

        self.last_injected = Some(known);
        self.last_signature = Some(signature);
        self.last_injected_at = Some(now);
        self.clear_pending();
        Some(payload)
    }

    fn clear_pending(&mut self) {
        self.pending.clear();
        self.written_at = None;
    }
}

/// Dedup identity of an entry: its whitespace-normalized text.
fn identity_set(entries: &[MemoryEntry]) -> HashSet<String> {
    entries
        .iter()
        .map(|e| normalize_ws(&e.text))
        .filter(|text| !text.is_empty())
        .collect()
}

/// Order-insensitive content signature: sort the (already deduped) texts,
/// hash them into one u64.
fn set_signature(known: &HashSet<String>) -> u64 {
    let mut texts: Vec<&String> = known.iter().collect();
    texts.sort();
    let mut hasher = DefaultHasher::new();
    for text in texts {
        text.hash(&mut hasher);
    }
    hasher.finish()
}

/// Gate (d) overlap: intersection over the larger of the two sets (jcode's
/// baseline), as integer math against [`OVERLAP_MIN_PCT`].
fn overlaps_enough(last: &HashSet<String>, next: &HashSet<String>) -> bool {
    if last.is_empty() || next.is_empty() {
        return false;
    }
    let shared = last.intersection(next).count();
    let baseline = last.len().max(next.len());
    // u64 math: usize overflow is theoretical here but the widening is free.
    (shared as u64) * 100 >= OVERLAP_MIN_PCT * baseline as u64
}

/// Collapse every whitespace run to one space and trim the ends — the one
/// normalization applied to entry text for rendering and dedup alike.
pub(crate) fn normalize_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
