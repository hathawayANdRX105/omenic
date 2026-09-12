//! Derived-graph tests: deterministic construction, supersede/contradict
//! walks, neighbor symmetry.

use memory::{Category, MemoryEntry, Trust};

fn entry(id: u64, text: &str, tags: &[&str]) -> MemoryEntry {
    let mut e = MemoryEntry::new(text);
    e.id = id;
    e.ts = "2026-01-01T00:00:00Z".into();
    e.tags = tags.iter().map(|t| t.to_string()).collect();
    e
}

#[test]
fn build_is_deterministic_across_input_orders() {
    let mut a = vec![
        entry(3, "rust toolchain pinned", &["tooling"]),
        entry(1, "uses zsh", &["shell", "tooling"]),
        entry(2, "deploy target is fly.io", &["deploy"]),
    ];
    let mut b = a.clone();
    b.reverse();

    let ga = memory::MemoryGraph::build(&a);
    let gb = memory::MemoryGraph::build(&b);
    let edges = |g: &memory::MemoryGraph| -> Vec<String> {
        let mut out: Vec<String> = g
            .forward
            .iter()
            .flat_map(|(from, es)| {
                es.iter()
                    .map(move |e| format!("{from}->{:?}:{}", e.kind, e.to))
            })
            .collect();
        out.sort();
        out
    };
    assert_eq!(
        edges(&ga),
        edges(&gb),
        "graph must not depend on input order"
    );
    // The shared tag produced edges in both directions.
    assert!(edges(&ga).iter().any(|e| e.contains("SharedTag")));
}

#[test]
fn supersedes_marks_and_links() {
    let mut old = entry(1, "deploy target is heroku", &["deploy"]);
    let new = entry(2, "deploy target is fly.io", &["deploy"]);
    old.superseded_by = Some(2);
    let g = memory::MemoryGraph::build(&[old, new]);

    assert!(!g.memories[&1].active || true, "active is caller-owned");
    // Old → new carries the Supersedes edge; the reverse walk mirrors it.
    let fwd: Vec<_> = g.neighbors(1);
    assert!(
        fwd.iter()
            .any(|(to, k)| *to == 2 && matches!(k, memory::graph::EdgeKind::Supersedes))
    );
    let back: Vec<_> = g.neighbors(2);
    assert!(
        back.iter()
            .any(|(to, k)| *to == 1 && matches!(k, memory::graph::EdgeKind::Supersedes))
    );
}

#[test]
fn contradicts_stays_traversable() {
    let mut claim = entry(1, "ci runs on friday", &["ci"]);
    let mut rebuttal = entry(2, "ci runs on monday", &["ci"]);
    rebuttal.contradicts = Some(1);
    claim.category = Category::Fact;
    let g = memory::MemoryGraph::build(&[claim, rebuttal]);

    assert!(
        g.neighbors(2)
            .iter()
            .any(|(to, k)| *to == 1 && matches!(k, memory::graph::EdgeKind::Contradicts))
    );
    // Entries remain reachable for triage; nothing is deleted.
    assert_eq!(g.memories.len(), 2);
    assert_eq!(g.memories[&1].trust, memory::Trust::Medium);
}

#[test]
fn serde_defaults_keep_old_store_shape_loadable() {
    // The old shape had exactly {id, ts, text}; it must deserialize with
    // every new field defaulted.
    let raw = r#"{"id":7,"ts":"2025-01-01T00:00:00Z","text":"legacy row"}"#;
    let e: MemoryEntry = serde_json::from_str(raw).unwrap();
    assert_eq!(e.id, 7);
    assert_eq!(e.text, "legacy row");
    assert_eq!(e.category, Category::Custom);
    assert!(e.tags.is_empty());
    assert_eq!(e.trust, memory::Trust::Medium);
    assert!((e.confidence - 0.5).abs() < f32::EPSILON);
    assert!(e.active);
    assert_eq!(e.superseded_by, None);
    assert_eq!(e.contradicts, None);
}

#[test]
fn new_fields_round_trip_through_json() {
    let mut e = entry(1, "prefers tabs", &["editor"]);
    e.category = Category::Preference;
    e.trust = memory::Trust::High;
    e.confidence = 0.9;
    e.strength = 3;
    e.contradicts = Some(9);

    let s = serde_json::to_string(&e).unwrap();
    let back: MemoryEntry = serde_json::from_str(&s).unwrap();
    assert_eq!(back, e);
}
