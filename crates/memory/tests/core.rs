//! Core memory behaviors: tokenization, recall ranking and cascade,
//! extractor output parsing, and the JSONL storage contract.

use memory::graph::MemoryGraph;
use memory::pipeline::extract_lines;
use memory::recall::{RecallPath, recall, tokenize};
use memory::{Category, Memory, MemoryEntry, Trust};

fn entry(
    id: u64,
    text: &str,
    active: bool,
    tags: &[&str],
    superseded_by: Option<u64>,
) -> MemoryEntry {
    MemoryEntry {
        id,
        ts: "2026-09-25T00:00:00Z".to_string(),
        text: text.to_string(),
        category: Category::Fact,
        tags: tags.iter().map(|s| s.to_string()).collect(),
        trust: Trust::Medium,
        confidence: 1.0,
        strength: 0,
        active,
        superseded_by,
        contradicts: None,
        embedding: None,
    }
}

#[test]
fn tokenize_ascii_words_and_digits() {
    assert_eq!(tokenize("Hello World 42"), vec!["hello", "world", "42"]);
}

#[test]
fn tokenize_cjk_chars_and_bigrams() {
    // Single chars first, then the adjacent bigram, in scan order.
    assert_eq!(tokenize("中文"), vec!["中", "文", "中文"]);
}

#[test]
fn recall_empty_query_yields_nothing() {
    let graph = MemoryGraph::build(&[entry(1, "postgres db", true, &[], None)]);
    assert!(recall(&graph, "", 5).is_empty());
    assert!(recall(&graph, "!!!", 5).is_empty());
}

#[test]
fn direct_ranking_prefers_higher_token_overlap() {
    let a = entry(1, "cargo build fast", true, &[], None);
    let b = entry(2, "cargo build test cargo", true, &[], None);
    let graph = MemoryGraph::build(&[a, b]);
    let hits = recall(&graph, "cargo build", 5);

    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].id, 2); // "cargo" appears twice in b
    assert!(matches!(hits[0].via, RecallPath::Direct));
    assert_eq!(hits[1].id, 1);
}

#[test]
fn inactive_entry_reachable_only_via_cascade() {
    // b is superseded by a: dead wording must not outrank the live entry,
    // but the graph walk can still surface it (discounted).
    let a = entry(1, "postgres db", true, &["db"], None);
    let b = entry(2, "sqlite db", false, &["db"], Some(1));
    let graph = MemoryGraph::build(&[a, b]);
    let hits = recall(&graph, "sqlite db", 5);

    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].id, 1);
    assert!(matches!(hits[0].via, RecallPath::Direct));
    assert_eq!(hits[1].id, 2);
    assert!(matches!(
        hits[1].via,
        RecallPath::Graph { from: 1, depth: 1 }
    ));
}

#[test]
fn recall_k_truncates_and_ties_break_by_id() {
    let entries = (1..=5)
        .map(|i| entry(i, format!("alpha n{i}").as_str(), true, &[], None))
        .collect::<Vec<_>>();
    let graph = MemoryGraph::build(&entries);
    let hits = recall(&graph, "alpha", 2);
    assert_eq!(hits.iter().map(|h| h.id).collect::<Vec<_>>(), vec![1, 2]);
}

#[test]
fn extract_lines_tolerates_numbering_and_skips_malformed() {
    let raw = "1. fact|db is postgres|high\n\
               preference|dark  mode|low\n\
               bogus|skip me\n\
               fact|   |high\n\
               correction|use  rustfmt";
    let out = extract_lines(raw);

    assert_eq!(out.len(), 3);
    assert_eq!(out[0].category, Category::Fact);
    assert_eq!(out[0].content, "db is postgres");
    assert_eq!(out[0].trust, Trust::High);
    assert_eq!(out[1].category, Category::Preference);
    assert_eq!(out[1].content, "dark mode");
    assert_eq!(out[1].trust, Trust::Low);
    // Trust missing from the last line -> medium; whitespace normalized.
    assert_eq!(out[2].category, Category::Correction);
    assert_eq!(out[2].content, "use rustfmt");
    assert_eq!(out[2].trust, Trust::Medium);
}

#[test]
fn storage_assigns_ids_latest_wins_and_supersedes() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = Memory::open(dir.path()).unwrap();

    assert_eq!(m.append(MemoryEntry::new("first")).unwrap(), 1);
    assert_eq!(m.append(MemoryEntry::new("second")).unwrap(), 2);

    // Same-id rewrite: the newer line shadows the older one on read.
    let mut upd = MemoryEntry::new("x");
    upd.id = 1;
    upd.text = "first-v2".into();
    m.remember_update(upd).unwrap();
    let list = m.list().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].text, "first-v2");

    let new_id = m.supersede(1, MemoryEntry::new("replacement")).unwrap();
    assert_eq!(new_id, 3);
    let retired = m.list().unwrap().into_iter().find(|e| e.id == 1).unwrap();
    assert!(!retired.active);
    assert_eq!(retired.superseded_by, Some(3));

    assert!(m.supersede(999, MemoryEntry::new("x")).is_err());
}
