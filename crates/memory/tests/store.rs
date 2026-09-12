//! Store-contract tests: flock + fsync JSONL, monotonic ids, torn-write
//! recovery, latest-wins duplicates.

use std::fs::OpenOptions;
use std::io::Write;

use memory::{Memory, MemoryEntry, MemoryError};

fn store() -> (Memory, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mem = Memory::open(tmp.path()).expect("open");
    (mem, tmp)
}

#[test]
fn disabled_is_noop() {
    let mut mem = Memory::disabled();
    assert!(!mem.enabled());
    mem.append(MemoryEntry::new("secret"))
        .expect("no-op append");
    assert!(mem.list().expect("no-op list").is_empty());
    assert!(mem.search("secret").expect("no-op search").is_empty());
}

#[test]
fn append_then_list_round_trip() {
    let (mut mem, _tmp) = store();
    assert!(mem.enabled());
    mem.append(MemoryEntry::new("user prefers tabs")).unwrap();
    mem.append(MemoryEntry::new("deploy target is fly.io"))
        .unwrap();

    let all = mem.list().unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].text, "user prefers tabs");
    assert_eq!(all[1].text, "deploy target is fly.io");
    assert!(
        all[0].ts.ends_with('Z'),
        "ts should be ISO-ish: {}",
        all[0].ts
    );
}

#[test]
fn ids_are_monotonic_across_handles() {
    let (mut mem, tmp) = store();
    mem.append(MemoryEntry::new("one")).unwrap();
    mem.append(MemoryEntry::new("two")).unwrap();
    // A fresh handle keeps counting from the stored max, not from 1.
    let mut again = Memory::open(tmp.path()).unwrap();
    again.append(MemoryEntry::new("three")).unwrap();

    let ids: Vec<u64> = again.list().unwrap().iter().map(|e| e.id).collect();
    assert_eq!(ids, vec![1, 2, 3]);
}

#[test]
fn caller_supplied_id_is_ignored() {
    let (mut mem, _tmp) = store();
    let mut entry = MemoryEntry::new("first");
    entry.id = 999;
    mem.append(entry).unwrap();
    assert_eq!(mem.list().unwrap()[0].id, 1);
}

#[test]
fn search_is_case_insensitive_substring() {
    let (mut mem, _tmp) = store();
    mem.append(MemoryEntry::new("Prefers Ripgrep over grep"))
        .unwrap();
    mem.append(MemoryEntry::new("uses zsh")).unwrap();

    assert_eq!(mem.search("RIPGREP").unwrap().len(), 1);
    assert_eq!(mem.search("zsh").unwrap()[0].text, "uses zsh");
    assert!(mem.search("nothing here").unwrap().is_empty());
    // Empty query matches everything.
    assert_eq!(mem.search("").unwrap().len(), 2);
}

#[test]
fn trailing_corrupt_line_is_trimmed() {
    let (mut mem, tmp) = store();
    mem.append(MemoryEntry::new("good")).unwrap();
    let path = tmp.path().join("memory.jsonl");
    // Torn write: partial line, no trailing newline.
    let mut f = OpenOptions::new().append(true).open(&path).unwrap();
    f.write_all(b"{\"id\":2,\"ts\":\"tor").unwrap();
    drop(f);

    let all = mem.list().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].text, "good");
    // The garbage is gone from disk, so the next append is clean.
    let raw = std::fs::read_to_string(&path).unwrap();
    assert_eq!(raw.lines().count(), 1, "trailing garbage left: {raw:?}");
    assert!(!raw.contains("\"id\":2"), "trailing garbage left: {raw:?}");
    mem.append(MemoryEntry::new("after")).unwrap();
    assert_eq!(mem.list().unwrap().len(), 2);
}

#[test]
fn append_repairs_torn_line_before_writing() {
    let (mut mem, tmp) = store();
    mem.append(MemoryEntry::new("good")).unwrap();
    let path = tmp.path().join("memory.jsonl");
    OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{\"")
        .unwrap();

    mem.append(MemoryEntry::new("fresh")).unwrap();
    // A second append without a list must remain clean too.
    mem.append(MemoryEntry::new("later")).unwrap();

    let all = mem.list().unwrap();
    assert_eq!(
        all.iter()
            .map(|entry| entry.text.as_str())
            .collect::<Vec<_>>(),
        ["good", "fresh", "later"]
    );
    assert_eq!(std::fs::read_to_string(path).unwrap().lines().count(), 3);
}

#[test]
fn torn_multibyte_utf8_does_not_block_recovery() {
    let (mut mem, tmp) = store();
    mem.append(MemoryEntry::new("good")).unwrap();
    let path = tmp.path().join("memory.jsonl");
    OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{\"id\":2,\"text\":\"caf\xc3")
        .unwrap();

    assert!(mem.list().is_ok());
    assert!(mem.append(MemoryEntry::new("after")).is_ok());
    assert_eq!(mem.list().unwrap().len(), 2);
}

#[test]
fn corrupt_middle_line_is_an_error() {
    let (mem, tmp) = store();
    let path = tmp.path().join("memory.jsonl");
    std::fs::write(&path, "not json\n{\"id\":1,\"ts\":\"t\",\"text\":\"ok\"}\n").unwrap();
    match mem.list() {
        Err(MemoryError::CorruptLine { line, .. }) => assert_eq!(line, 1),
        other => panic!("expected CorruptLine, got {other:?}"),
    }
}

#[test]
fn duplicate_id_latest_wins() {
    let (mem, tmp) = store();
    let path = tmp.path().join("memory.jsonl");
    std::fs::write(
        &path,
        "{\"id\":1,\"ts\":\"t\",\"text\":\"old\"}\n{\"id\":1,\"ts\":\"t\",\"text\":\"new\"}\n",
    )
    .unwrap();
    let all = mem.list().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].text, "new");
}

#[test]
fn empty_store_lists_nothing() {
    let (mem, _tmp) = store();
    assert!(mem.list().unwrap().is_empty());
}

#[test]
fn append_returns_the_assigned_id() {
    let (mut mem, tmp) = store();
    assert_eq!(mem.append(MemoryEntry::new("one")).unwrap(), 1);
    assert_eq!(mem.append(MemoryEntry::new("two")).unwrap(), 2);
    // A fresh handle continues from the stored max.
    let mut again = Memory::open(tmp.path()).unwrap();
    assert_eq!(again.append(MemoryEntry::new("three")).unwrap(), 3);
}

#[test]
fn disabled_append_returns_zero_without_writing() {
    let mut mem = Memory::disabled();
    assert_eq!(mem.append(MemoryEntry::new("x")).unwrap(), 0);
    assert!(mem.list().unwrap().is_empty());
}
