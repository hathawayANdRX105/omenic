//! MEM-2 injection pipeline: the four gate layers (freshness / identical
//! set / signature / overlap), payload formatting, and the update channel
//! (`remember_update`, `supersede`). All clocks are injected as plain unix
//! seconds — nothing here reads the wall clock.

use std::fs::OpenOptions;
use std::io::Write;

use memory::{Category, InjectionBuffer, Memory, MemoryEntry, MemoryError, format_injection};

fn store() -> (Memory, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mem = Memory::open(tmp.path()).expect("open");
    (mem, tmp)
}

fn entry(text: &str, category: Category) -> MemoryEntry {
    let mut e = MemoryEntry::new(text);
    e.category = category;
    e
}

fn fact(text: &str) -> MemoryEntry {
    entry(text, Category::Fact)
}

#[test]
fn freshness_expiry_drops_pending() {
    let mut buf = InjectionBuffer::new();
    buf.set_memories(vec![fact("alpha")], 1000);
    assert!(buf.take_ready(1119).is_some(), "119s old is still fresh");

    // A payload that ages past the 120s window is dropped, not injected late.
    buf.set_memories(vec![fact("beta")], 2000);
    assert_eq!(buf.take_ready(2121), None);
    assert!(buf.is_empty(), "expiry must clear the buffer");
    assert_eq!(buf.take_ready(2300), None, "cleared buffer stays empty");
}

#[test]
fn signature_dedup_suppresses_same_content_within_window() {
    let mut buf = InjectionBuffer::new();
    buf.set_memories(vec![fact("alpha"), fact("beta")], 1000);
    assert!(buf.take_ready(1000).is_some());

    // Same content re-set inside the 90s window — even reordered, with
    // rebuilt entries — is held back.
    buf.set_memories(vec![fact("beta"), fact("alpha")], 1030);
    assert_eq!(buf.take_ready(1030), None);

    // A genuinely different set still gets through inside the window.
    buf.set_memories(vec![fact("gamma")], 1050);
    assert!(buf.take_ready(1050).is_some());
}

#[test]
fn overlap_gate_suppresses_reshuffle_until_window_passes() {
    let mut buf = InjectionBuffer::new();
    buf.set_memories(
        vec![fact("a"), fact("b"), fact("c"), fact("d"), fact("e")],
        1000,
    );
    assert!(buf.take_ready(1000).is_some());

    // 4 of 5 kept + 1 new = 80% overlap (baseline: the larger set), inside
    // the 180s window → parked for later, not dropped.
    buf.set_memories(
        vec![fact("a"), fact("b"), fact("c"), fact("d"), fact("f")],
        1170,
    );
    assert_eq!(buf.take_ready(1170), None);

    // Past the 180s window the same set is news again.
    buf.set_memories(
        vec![fact("a"), fact("b"), fact("c"), fact("d"), fact("f")],
        1500,
    );
    assert!(buf.take_ready(1500).is_some());
}

#[test]
fn overlap_below_threshold_injects_inside_window() {
    let mut buf = InjectionBuffer::new();
    buf.set_memories(
        vec![fact("a"), fact("b"), fact("c"), fact("g"), fact("h")],
        1000,
    );
    assert!(buf.take_ready(1000).is_some());

    // 2 of 5 shared = 40% < 80%: a genuinely new mix, even right after the
    // previous injection.
    buf.set_memories(
        vec![fact("a"), fact("b"), fact("x"), fact("y"), fact("z")],
        1050,
    );
    let payload = buf.take_ready(1050).expect("40% overlap must inject");
    assert!(payload.contains("3. x"), "new items in payload: {payload}");
    assert!(payload.contains("5. z"), "new items in payload: {payload}");
}

#[test]
fn format_groups_sections_in_fixed_order() {
    let mems = vec![
        entry("note about   kbd layout", Category::Custom),
        entry("deploys to fly.io", Category::Fact),
        entry("fix\tdeploy  target ", Category::Correction),
        entry("service named atlas", Category::Entity),
        entry("prefer spaces over tabs", Category::Preference),
    ];
    assert_eq!(
        format_injection(&mems),
        "<system-reminder>\n# Memory\n## Corrections\n1. fix deploy target\n\
         ## Preferences\n1. prefer spaces over tabs\n\
         ## Facts\n1. deploys to fly.io\n\
         ## Entities\n1. service named atlas\n\
         ## Notes\n1. note about kbd layout\n\
         </system-reminder>"
    );
}

#[test]
fn format_numbers_within_section_and_omits_empty_sections() {
    let mems = vec![fact("second fact"), fact("first fact"), fact("third fact")];
    assert_eq!(
        format_injection(&mems),
        "<system-reminder>\n# Memory\n## Facts\n1. second fact\n2. first fact\n\
         3. third fact\n</system-reminder>"
    );
}

#[test]
fn format_returns_empty_for_nothing_injectable() {
    assert_eq!(format_injection(&[]), "");
    assert_eq!(format_injection(&[entry("   \t ", Category::Fact)]), "");

    let mut buf = InjectionBuffer::new();
    assert_eq!(buf.take_ready(999), None, "empty buffer takes nothing");
    buf.set_memories(vec![], 0);
    assert_eq!(buf.take_ready(1), None);
    buf.set_memories(vec![entry("  ", Category::Fact)], 0);
    assert_eq!(buf.take_ready(1), None);
    assert!(buf.is_empty(), "blank-only payload must not stick around");
}

#[test]
fn remember_update_keeps_id_and_wins_latest() {
    let (mut mem, tmp) = store();
    mem.append(MemoryEntry::new("user prefers tabs")).unwrap();
    let stored = mem.list().unwrap()[0].clone();

    let mut updated = stored.clone();
    updated.text = "user prefers spaces".into();
    updated.category = Category::Preference;
    updated.confidence = 0.9;
    mem.remember_update(updated).unwrap();

    let all = mem.list().unwrap();
    assert_eq!(all.len(), 1, "same id must shadow, not duplicate");
    assert_eq!(all[0].id, stored.id, "caller-supplied id is preserved");
    assert_eq!(all[0].text, "user prefers spaces");
    assert_eq!(all[0].category, Category::Preference);
    assert!((all[0].confidence - 0.9).abs() < f32::EPSILON);

    // Append-only file: the corrected row keeps its history line.
    let raw = std::fs::read_to_string(tmp.path().join("memory.jsonl")).unwrap();
    assert_eq!(raw.lines().count(), 2);
}

#[test]
fn remember_update_repairs_torn_trailing_line() {
    let (mut mem, tmp) = store();
    mem.append(MemoryEntry::new("good")).unwrap();
    let path = tmp.path().join("memory.jsonl");
    OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{\"")
        .unwrap();

    let stored = mem.list().unwrap()[0].clone();
    let mut updated = stored.clone();
    updated.text = "corrected".into();
    mem.remember_update(updated).unwrap();

    let all = mem.list().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].text, "corrected");
    assert_eq!(std::fs::read_to_string(path).unwrap().lines().count(), 2);
}

#[test]
fn supersede_marks_old_row_and_links_the_new_one() {
    let (mut mem, _tmp) = store();
    mem.append(MemoryEntry::new("deploy to fly.io")).unwrap();
    let old = &mem.list().unwrap()[0];
    let old_id = old.id;
    let old_ts = old.ts.clone();

    let mut new = MemoryEntry::new("deploy to hatchbox");
    new.category = Category::Correction;
    let new_id = mem.supersede(old_id, new).expect("supersede");
    assert_eq!(new_id, old_id + 1, "new row gets the next id");

    let all = mem.list().unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].id, old_id);
    assert!(!all[0].active, "old row must be retired");
    assert_eq!(all[0].superseded_by, Some(new_id));
    assert_eq!(all[0].text, "deploy to fly.io", "other fields stay");
    assert_eq!(all[0].ts, old_ts, "other fields stay");
    assert_eq!(all[1].id, new_id);
    assert!(all[1].active);
    assert_eq!(all[1].superseded_by, None);
    assert_eq!(all[1].text, "deploy to hatchbox");
}

#[test]
fn supersede_unknown_id_errors_without_writing() {
    let (mut mem, _tmp) = store();
    mem.append(MemoryEntry::new("only")).unwrap();
    match mem.supersede(999, MemoryEntry::new("ghost")) {
        Err(MemoryError::UnknownId { id }) => assert_eq!(id, 999),
        other => panic!("expected UnknownId, got {other:?}"),
    }
    assert_eq!(mem.list().unwrap().len(), 1, "nothing written on error");
}

#[test]
fn disabled_store_is_noop_for_update_channel() {
    let mut mem = Memory::disabled();
    mem.remember_update(MemoryEntry::new("x")).unwrap();
    assert_eq!(mem.supersede(1, MemoryEntry::new("y")).unwrap(), 0);
    assert!(mem.list().unwrap().is_empty());
}
