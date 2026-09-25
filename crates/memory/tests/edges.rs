//! Storage edge cases (corrupt lines), search, graph view, and cosine
//! semantics.

use std::io::Write;

use memory::embed::cosine;
use memory::graph::EdgeKind;
use memory::{Memory, MemoryEntry, MemoryError, Trust};

#[test]
fn list_trims_corrupt_trailing_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("memory.jsonl");
    let mut m = Memory::open(dir.path()).unwrap();
    m.append(MemoryEntry::new("one")).unwrap();
    m.append(MemoryEntry::new("two")).unwrap();
    // Simulate a torn write: a partial JSON line without its newline.
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{\"id\": 3, \"ts\": \"t\"")
        .unwrap();

    let list = m.list().unwrap();
    assert_eq!(list.len(), 2);
    assert!(
        !std::fs::read_to_string(&path)
            .unwrap()
            .contains("\"id\": 3")
    );
}

#[test]
fn list_rejects_corrupt_middle_line() {
    let dir = tempfile::tempdir().unwrap();
    let m = Memory::open(dir.path()).unwrap();
    let path = dir.path().join("memory.jsonl");

    let first = MemoryEntry {
        id: 1,
        text: "one".into(),
        ..MemoryEntry::new("one")
    };
    let second = MemoryEntry {
        id: 2,
        text: "two".into(),
        ..MemoryEntry::new("two")
    };
    std::fs::write(
        &path,
        format!(
            "{}\nnot json\n{}\n",
            serde_json::to_string(&first).unwrap(),
            serde_json::to_string(&second).unwrap()
        ),
    )
    .unwrap();

    let err = m.list().unwrap_err();
    assert!(matches!(err, MemoryError::CorruptLine { line: 2, .. }));
}

#[test]
fn search_is_case_insensitive_substring() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = Memory::open(dir.path()).unwrap();
    let mut e = MemoryEntry::new("Cargo build flags");
    e.trust = Trust::High;
    m.append(e).unwrap();

    assert_eq!(m.search("cargo").unwrap().len(), 1);
    assert_eq!(m.search("BUILD FLAGS").unwrap().len(), 1);
    assert_eq!(m.search("rustc").unwrap().len(), 0);
}

#[test]
fn graph_view_exposes_tag_neighbors() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = Memory::open(dir.path()).unwrap();

    let mut a = MemoryEntry::new("postgres");
    a.tags = vec!["db".into()];
    m.append(a).unwrap();
    let mut b = MemoryEntry::new("sqlite");
    b.tags = vec!["db".into()];
    m.append(b).unwrap();

    let graph = m.graph_view().unwrap();
    assert!(
        graph
            .neighbors(1)
            .iter()
            .any(|(to, kind)| { *to == 2 && matches!(kind, EdgeKind::SharedTag(_)) })
    );
}

#[test]
fn cosine_semantics() {
    assert_eq!(cosine(&[1.0, 0.0], &[1.0, 0.0]), Some(1.0));
    assert_eq!(cosine(&[1.0, 0.0], &[0.0, 1.0]), Some(0.0));
    // Length mismatch, zero vector, and empty vectors: undefined, not 0.
    assert!(cosine(&[1.0], &[1.0, 0.0]).is_none());
    assert!(cosine(&[0.0, 0.0], &[0.0, 0.0]).is_none());
    assert!(cosine(&[], &[]).is_none());
}
