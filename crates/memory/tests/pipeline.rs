//! Write-pipeline tests: `Memory::remember` dedup/reinforce driven by a
//! deterministic fake embedder, the DEDUP_COSINE boundary, and the
//! `ExtractionTriggers` decision table.

use std::collections::BTreeMap;

use memory::{
    Category, DEDUP_COSINE, EmbedError, Embedder, ExtractionTriggers, Memory, RememberOutcome,
    cosine,
};

// ===== fakes =====

/// Deterministic embedder: each scripted text maps to a fixed vector; an
/// unscripted text panics so test intent stays visible.
struct MapEmbedder {
    vectors: BTreeMap<&'static str, Vec<f32>>,
}

impl MapEmbedder {
    fn new(pairs: Vec<(&'static str, Vec<f32>)>) -> MapEmbedder {
        MapEmbedder {
            vectors: pairs.into_iter().collect(),
        }
    }
}

impl Embedder for MapEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        Ok(texts
            .iter()
            .map(|t| {
                self.vectors
                    .get(t.as_str())
                    .cloned()
                    .unwrap_or_else(|| panic!("no scripted vector for {t:?}"))
            })
            .collect())
    }
}

/// Panics if the pipeline ever calls it — proves the disabled handle is a
/// true no-op.
struct MustNotEmbed;

impl Embedder for MustNotEmbed {
    fn embed(&self, _: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        panic!("embedder must not run on a disabled handle")
    }
}

/// Unit vector whose cosine with `[1, 0, 0]` is exactly `c`.
fn cosine_with_x_axis(c: f32) -> Vec<f32> {
    vec![c, (1.0 - c * c).sqrt(), 0.0]
}

const X_AXIS: &[f32] = &[1.0, 0.0, 0.0];

fn store() -> (Memory, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("tempdir");
    (Memory::open(tmp.path()).expect("open"), tmp)
}

// ===== remember =====

#[test]
fn remember_stores_new_entry_with_embedding() {
    let (mut mem, _tmp) = store();
    let embedder = MapEmbedder::new(vec![("user prefers tabs".into(), X_AXIS.to_vec())]);

    let out = mem
        .remember(
            "user prefers tabs",
            Category::Preference,
            vec!["editor".into()],
            &embedder,
        )
        .unwrap();

    assert_eq!(out, RememberOutcome::Stored { id: 1 });
    let all = mem.list().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].text, "user prefers tabs");
    assert_eq!(all[0].category, Category::Preference);
    assert_eq!(all[0].tags, vec!["editor".to_string()]);
    assert_eq!(all[0].embedding.as_deref(), Some(X_AXIS));
    assert_eq!(all[0].strength, 0);
}

#[test]
fn remember_reinforces_near_duplicate_instead_of_appending() {
    let (mut mem, _tmp) = store();
    let embedder = MapEmbedder::new(vec![
        ("user prefers tabs over spaces".into(), X_AXIS.to_vec()),
        (
            "user likes tabs more than spaces".into(),
            cosine_with_x_axis(0.95),
        ),
    ]);

    let first = mem
        .remember(
            "user prefers tabs over spaces",
            Category::Preference,
            vec!["editor".into()],
            &embedder,
        )
        .unwrap();
    assert_eq!(first, RememberOutcome::Stored { id: 1 });

    let second = mem
        .remember(
            "user likes tabs more than spaces",
            Category::Custom,
            vec![],
            &embedder,
        )
        .unwrap();
    assert_eq!(second, RememberOutcome::Reinforced { id: 1 });

    // No new row; the original is strengthened and re-embedded, and keeps
    // its original text/category/tags/trust.
    let all = mem.list().unwrap();
    assert_eq!(all.len(), 1, "reinforce must not append a row");
    let entry = &all[0];
    assert_eq!(entry.id, 1);
    assert_eq!(entry.text, "user prefers tabs over spaces");
    assert_eq!(entry.category, Category::Preference);
    assert_eq!(entry.tags, vec!["editor".to_string()]);
    assert_eq!(entry.strength, 1);
    assert_eq!(
        entry.embedding.as_deref(),
        Some(cosine_with_x_axis(0.95).as_slice()),
        "embedding must be overwritten with the freshly computed one"
    );

    // And it reinforces again on a third hit.
    let third = mem
        .remember(
            "user likes tabs more than spaces",
            Category::Custom,
            vec![],
            &embedder,
        )
        .unwrap();
    assert_eq!(third, RememberOutcome::Reinforced { id: 1 });
    assert_eq!(mem.list().unwrap()[0].strength, 2);
}

#[test]
fn remember_stores_different_content_as_new_row() {
    let (mut mem, _tmp) = store();
    let embedder = MapEmbedder::new(vec![
        ("user prefers tabs".into(), X_AXIS.to_vec()),
        ("deploy target is fly.io".into(), vec![0.0, 1.0, 0.0]),
    ]);

    mem.remember("user prefers tabs", Category::Custom, vec![], &embedder)
        .unwrap();
    let out = mem
        .remember("deploy target is fly.io", Category::Fact, vec![], &embedder)
        .unwrap();

    assert_eq!(out, RememberOutcome::Stored { id: 2 });
    assert_eq!(mem.list().unwrap().len(), 2);
}

#[test]
fn remember_disabled_is_a_noop_without_embedding() {
    let mut mem = Memory::disabled();
    let out = mem
        .remember("anything", Category::Custom, vec![], &MustNotEmbed)
        .unwrap();
    assert_eq!(out, RememberOutcome::Stored { id: 0 });
    assert!(mem.list().unwrap().is_empty());
}

#[test]
fn dedup_cosine_boundary_above_reinforces_below_stores() {
    // Just above the 0.90 line: reinforce.
    let (mut mem, _tmp) = store();
    let above = cosine_with_x_axis(DEDUP_COSINE + 0.005);
    let embedder = MapEmbedder::new(vec![
        ("first".into(), X_AXIS.to_vec()),
        ("second".into(), above),
    ]);
    mem.remember("first", Category::Custom, vec![], &embedder)
        .unwrap();
    let out = mem
        .remember("second", Category::Custom, vec![], &embedder)
        .unwrap();
    assert_eq!(out, RememberOutcome::Reinforced { id: 1 });
    assert_eq!(mem.list().unwrap().len(), 1);

    // Just below the line: a genuinely new entry.
    let (mut mem, _tmp) = store();
    let below = cosine_with_x_axis(DEDUP_COSINE - 0.005);
    assert!(cosine(X_AXIS, &below).unwrap() < DEDUP_COSINE);
    let embedder = MapEmbedder::new(vec![
        ("first".into(), X_AXIS.to_vec()),
        ("second".into(), below),
    ]);
    mem.remember("first", Category::Custom, vec![], &embedder)
        .unwrap();
    let out = mem
        .remember("second", Category::Custom, vec![], &embedder)
        .unwrap();
    assert_eq!(out, RememberOutcome::Stored { id: 2 });
    assert_eq!(mem.list().unwrap().len(), 2);
}

#[test]
fn entries_without_embedding_are_invisible_to_dedup() {
    let (mut mem, _tmp) = store();
    let embedder = MapEmbedder::new(vec![("same text".into(), X_AXIS.to_vec())]);
    // A plain append carries no embedding.
    let mut seeded = memory::MemoryEntry::new("same text");
    seeded.embedding = None;
    mem.append(seeded).unwrap();

    // Even an identical text cannot cosine-match a vectorless row, so it is
    // stored as a new entry.
    let out = mem
        .remember("same text", Category::Custom, vec![], &embedder)
        .unwrap();
    assert_eq!(out, RememberOutcome::Stored { id: 2 });
    assert_eq!(mem.list().unwrap().len(), 2);
}

// ===== extraction triggers =====

const TOPIC_A: &[f32] = &[1.0, 0.0, 0.0];
const TOPIC_B: &[f32] = &[0.0, 1.0, 0.0]; // cosine(A, B) = 0 < 0.3

#[test]
fn session_ending_always_fires() {
    let mut t = ExtractionTriggers::new();
    assert!(t.should_extract(0, None, true));
    assert!(t.should_extract(0, Some(TOPIC_A), true));

    // Even mid-session, far under every interval.
    t.mark_extracted(10, Some(TOPIC_A));
    assert!(t.should_extract(11, Some(TOPIC_A), true));
}

#[test]
fn fresh_state_never_fires_before_first_extraction() {
    // Design: before the first mark_extracted the caller runs its startup
    // extraction itself; until then only session_ending triggers.
    let t = ExtractionTriggers::new();
    assert!(!t.should_extract(0, None, false));
    assert!(!t.should_extract(3, Some(TOPIC_B), false));
    assert!(!t.should_extract(11, Some(TOPIC_B), false));
}

#[test]
fn topic_change_fires_only_after_min_turns() {
    let mut t = ExtractionTriggers::new();
    t.mark_extracted(0, Some(TOPIC_A));

    // Turn 3: topic jumped but the interval is too short — no extraction.
    assert!(!t.should_extract(3, Some(TOPIC_B), false));
    // Turn 4 (= TOPIC_MIN_TURNS): topic change + interval -> extract.
    assert!(t.should_extract(4, Some(TOPIC_B), false));

    // Interval is measured from the last extraction, not from the topic
    // change: extraction at turn 10, jump seen at turn 12 -> only 2 turns.
    let mut t = ExtractionTriggers::new();
    t.mark_extracted(10, Some(TOPIC_A));
    assert!(!t.should_extract(12, Some(TOPIC_B), false));
    assert!(t.should_extract(14, Some(TOPIC_B), false));
}

#[test]
fn periodic_cadence_fires_at_twelve_turns() {
    let mut t = ExtractionTriggers::new();
    t.mark_extracted(0, Some(TOPIC_A));

    // Same topic, so only the periodic rule can fire.
    assert!(!t.should_extract(11, Some(TOPIC_A), false));
    assert!(t.should_extract(12, Some(TOPIC_A), false));
}

#[test]
fn missing_topic_skips_topic_rule_but_keeps_periodic() {
    let mut t = ExtractionTriggers::new();
    t.mark_extracted(0, Some(TOPIC_A));

    // No current embedding: the topic rule cannot judge, so turn 5 does not
    // fire even though a topic change would have.
    assert!(!t.should_extract(5, None, false));
    // The periodic rule is independent of the topic vector.
    assert!(t.should_extract(12, None, false));

    // No last topic either (extraction ran without an embedding): still no
    // topic rule.
    let mut t = ExtractionTriggers::new();
    t.mark_extracted(0, None);
    assert!(!t.should_extract(5, Some(TOPIC_B), false));
    assert!(t.should_extract(12, Some(TOPIC_B), false));
}

#[test]
fn incomparable_topic_vectors_are_not_a_topic_change() {
    let mut t = ExtractionTriggers::new();
    t.mark_extracted(0, Some(TOPIC_A));
    // Dimension mismatch -> cosine None -> conservative: no topic change.
    assert!(!t.should_extract(5, Some(&[1.0, 0.0, 0.0, 0.0][..]), false));
    // Zero vector likewise.
    assert!(!t.should_extract(5, Some(&[0.0, 0.0, 0.0][..]), false));
}

#[test]
fn similar_topic_is_not_a_change() {
    let mut t = ExtractionTriggers::new();
    t.mark_extracted(0, Some(TOPIC_A));
    // cosine 0.95 >= 0.3: same topic, only the periodic rule can fire.
    let similar = cosine_with_x_axis(0.95);
    assert!(!t.should_extract(5, Some(similar.as_slice()), false));
    assert!(t.should_extract(12, Some(similar.as_slice()), false));
}

// ===== extraction parsing =====

#[test]
fn extract_lines_parses_strict_format() {
    use memory::pipeline::extract_lines;
    use memory::{Category, Trust};

    let lines =
        extract_lines("fact|deploy target is fly.io|high\ncorrection|端口是 8026 不是 3000|low");
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].category, Category::Fact);
    assert_eq!(lines[0].trust, Trust::High);
    assert_eq!(lines[0].content, "deploy target is fly.io");
    assert_eq!(lines[1].category, Category::Correction);
    assert_eq!(lines[1].trust, Trust::Low);
    assert_eq!(lines[1].content, "端口是 8026 不是 3000");
}

#[test]
fn extract_lines_tolerates_numbering_and_missing_trust() {
    use memory::Trust;
    use memory::pipeline::extract_lines;

    let lines = extract_lines("1. preference|likes tabs\n2. fact|deploys on friday|medium\n");
    assert_eq!(lines.len(), 2);
    // Missing trust defaults to medium.
    assert_eq!(lines[0].trust, Trust::Medium);
    assert_eq!(lines[0].category, memory::Category::Preference);
    assert_eq!(lines[1].content, "deploys on friday");
}

#[test]
fn extract_lines_skips_malformed_and_empty() {
    use memory::pipeline::extract_lines;

    let lines =
        extract_lines("no pipes here\nfact||high\nunknown-kind|text|low\n\nfact|real content\n");
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].content, "real content");
}
