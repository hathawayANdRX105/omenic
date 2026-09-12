//! Memory write pipeline: dedup-on-remember and extraction triggering.
//!
//! [`Memory::remember`] computes an embedding for the incoming text, finds
//! the most similar active entry, and reinforces it instead of appending a
//! near-duplicate (jcode `remember_project` lineage, with the dedup cosine
//! tightened to 0.90). [`ExtractionTriggers`] is the sidecar-free trigger
//! model from jcode `memory_agent`: extract when the topic jumped, on a
//! periodic cadence, or at session end — no LLM in the decision chain.

use crate::{Category, EmbedError, Embedder, Memory, MemoryEntry, MemoryError, cosine, now_iso};

/// Cosine at or above which an incoming text is a duplicate of an existing
/// entry: reinforce it, do not append. jcode used `STORAGE_DEDUP_THRESHOLD`
/// = 0.85; MEM-4a tightens this to 0.90 so near-duplicates are appended as
/// genuinely new statements rather than silently merged.
pub const DEDUP_COSINE: f32 = 0.90;

/// Cosine below which the current topic counts as "changed" versus the last
/// extraction's topic. jcode `memory_agent::TOPIC_CHANGE_THRESHOLD`.
pub const TOPIC_CHANGE_COSINE: f32 = 0.3;

/// A topic change only fires extraction when at least this many turns have
/// passed since the last one (jcode `memory_agent::MIN_TURNS_FOR_EXTRACTION`).
pub const TOPIC_MIN_TURNS: u32 = 4;

/// Extract periodically every this many turns even without a topic change
/// (jcode `memory_agent::PERIODIC_EXTRACTION_INTERVAL`).
pub const PERIODIC_TURNS: u32 = 12;

/// What [`Memory::remember`] did with the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RememberOutcome {
    /// Appended as a new entry with the store-assigned id.
    Stored { id: u64 },
    /// Matched an existing active entry (cosine >= [`DEDUP_COSINE`]) and
    /// reinforced it in place; `id` is that entry's.
    Reinforced { id: u64 },
}

impl RememberOutcome {
    /// The id of the stored or reinforced entry.
    pub fn id(&self) -> u64 {
        match self {
            RememberOutcome::Stored { id } | RememberOutcome::Reinforced { id } => *id,
        }
    }
}

impl Memory {
    /// Remember `text` through the write pipeline: embed it, dedup against
    /// active entries that carry an embedding, then either reinforce the
    /// best match or append a new entry.
    ///
    /// Reinforcing keeps the entry's text, category, tags and trust, and
    /// refreshes `ts` to now, bumps `strength` by one, and overwrites the
    /// stored embedding with the freshly computed one (latest phrasing wins
    /// in vector space). The new-entry case stamps the same defaults as a
    /// plain append (`Trust::Medium` / `Category::Custom`).
    ///
    /// The disabled handle short-circuits before the embedder runs: no
    /// network call, no write, `Stored { id: 0 }` — same contract as the
    /// other no-ops.
    pub fn remember(
        &mut self,
        text: impl Into<String>,
        category: Category,
        tags: Vec<String>,
        embedder: &dyn Embedder,
    ) -> Result<RememberOutcome, MemoryError> {
        if !self.enabled() {
            return Ok(RememberOutcome::Stored { id: 0 });
        }
        let text = text.into();

        let vectors = embedder.embed(std::slice::from_ref(&text))?;
        let embedding = vectors
            .into_iter()
            .next()
            .ok_or(EmbedError::Count { want: 1, got: 0 })?;

        // Best active match by cosine; entries without an embedding (plain
        // appends, pre-pipeline rows) are invisible to dedup but stay listed.
        // Collect (entry, sim) pairs and take the true maximum with a total
        // order — filtering before max would hide the runner-up from anyone
        // tuning the threshold, and partial_cmp on f32 needs the NaN-free
        // guarantee cosine() provides, so total_cmp states it outright.
        let best = self
            .list()?
            .into_iter()
            .filter(|e| e.active)
            .filter_map(|e| cosine(&embedding, e.embedding.as_deref()?).map(|sim| (e, sim)))
            .max_by(|a, b| a.1.total_cmp(&b.1));

        if let Some((mut existing, sim)) = best {
            if sim >= DEDUP_COSINE {
                let id = existing.id;
                existing.ts = now_iso();
                existing.strength += 1;
                existing.embedding = Some(embedding);
                self.remember_update(existing)?;
                return Ok(RememberOutcome::Reinforced { id });
            }
        }

        let mut entry = MemoryEntry::new(text);
        entry.category = category;
        entry.tags = tags;
        entry.embedding = Some(embedding);
        let id = self.append(entry)?;
        Ok(RememberOutcome::Stored { id })
    }
}

/// Sidecar-free extraction trigger state for one session (jcode
/// `memory_agent` session-state lineage). The caller owns the turn counter
/// and topic embedding; this struct only remembers where the last extraction
/// happened and answers `should we extract now?`.
///
/// The topic vector should be the embedding of the current conversation
/// context; without one (`None`) the topic-change rule is skipped —
/// extraction then happens only on the periodic cadence or at session end.
#[derive(Debug, Clone, Default)]
pub struct ExtractionTriggers {
    /// Absolute turn index of the last extraction; `None` until the first
    /// [`Self::mark_extracted`]. The "turns since last" the thresholds talk
    /// about is derived from this and the caller's counter.
    last_turn: Option<u32>,
    /// Topic embedding at the last extraction.
    last_topic: Option<Vec<f32>>,
}

impl ExtractionTriggers {
    /// Fresh state: nothing extracted yet. Until the first
    /// [`Self::mark_extracted`] only `session_ending` fires — callers are
    /// expected to run their startup extraction and seed the state then.
    pub fn new() -> ExtractionTriggers {
        ExtractionTriggers::default()
    }

    /// Should an extraction run now?
    ///
    /// - `session_ending` always fires (never delay a session flush).
    /// - Topic changed (cosine of `current_topic` vs the last topic
    ///   `<` [`TOPIC_CHANGE_COSINE`]) and at least [`TOPIC_MIN_TURNS`] turns
    ///   since the last extraction.
    /// - At least [`PERIODIC_TURNS`] turns since the last extraction.
    ///
    /// A missing topic (`None`) or incomparable vectors (dimension mismatch,
    /// zero vector — [`cosine`] returns `None`) never count as a topic
    /// change; the periodic and session-end rules still apply.
    pub fn should_extract(
        &self,
        turn_count: u32,
        current_topic: Option<&[f32]>,
        session_ending: bool,
    ) -> bool {
        if session_ending {
            return true;
        }
        let Some(last_turn) = self.last_turn else {
            return false;
        };
        let since = turn_count.saturating_sub(last_turn);

        // Topic switch: enough turns have passed since the last extraction
        // AND the conversation moved (cosine below the threshold). Kept as
        // its own statement so the periodic rule below stays independent.
        if since >= TOPIC_MIN_TURNS
            && let (Some(current), Some(last)) = (current_topic, self.last_topic.as_deref())
            && matches!(cosine(current, last), Some(sim) if sim < TOPIC_CHANGE_COSINE)
        {
            return true;
        }
        // Periodic cadence: regardless of topic movement.
        since >= PERIODIC_TURNS
    }

    /// Record that an extraction just ran at `turn_count` with `topic` as
    /// the (embedded) conversation context.
    pub fn mark_extracted(&mut self, turn_count: u32, topic: Option<&[f32]>) {
        self.last_turn = Some(turn_count);
        self.last_topic = topic.map(|t| t.to_vec());
    }
}
