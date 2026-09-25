//! Injection pipeline: `<system-reminder>` formatting and the four
//! suppression gates on the caller-owned [`InjectionBuffer`].

use memory::inject::{InjectionBuffer, format_injection};
use memory::{Category, MemoryEntry};

fn entry(id: u64, category: Category, text: &str) -> MemoryEntry {
    let mut e = MemoryEntry::new(text);
    e.id = id;
    e.category = category;
    e
}

#[test]
fn format_injection_fixed_section_order() {
    let entries = vec![
        entry(1, Category::Custom, "a note"),
        entry(2, Category::Fact, "fact one"),
        entry(3, Category::Fact, "  fact two  "),
        entry(4, Category::Correction, "fix"),
    ];
    let payload = format_injection(&entries);

    // Preference/Entity sections are omitted; numbering restarts per section;
    // whitespace is normalized per entry.
    assert_eq!(
        payload,
        "<system-reminder>\n\
         # Memory\n\
         ## Corrections\n\
         1. fix\n\
         ## Facts\n\
         1. fact one\n\
         2. fact two\n\
         ## Notes\n\
         1. a note\n\
         </system-reminder>"
    );
}

#[test]
fn format_injection_empty_when_nothing_renderable() {
    assert_eq!(format_injection(&[]), "");
    let blank = entry(1, Category::Fact, "   ");
    assert_eq!(format_injection(&[blank]), "");
}

#[test]
fn fresh_pending_set_injects() {
    let mut buf = InjectionBuffer::new();
    buf.set_memories(vec![entry(1, Category::Fact, "fact text")], 1000);
    let payload = buf.take_ready(1001).expect("fresh set injects");
    assert!(payload.contains("## Facts"));
    assert!(payload.contains("fact text"));
    assert!(buf.is_empty());
}

#[test]
fn stale_pending_set_is_dropped() {
    let mut buf = InjectionBuffer::new();
    buf.set_memories(vec![entry(1, Category::Fact, "x")], 1000);
    assert!(buf.take_ready(1120).is_none()); // 120s exactly is no longer fresh
    assert!(buf.is_empty());
    assert!(buf.take_ready(1121).is_none());
}

#[test]
fn blank_pending_set_is_dropped() {
    let mut buf = InjectionBuffer::new();
    buf.set_memories(vec![entry(1, Category::Fact, "   ")], 0);
    assert!(buf.take_ready(1).is_none());
    assert!(buf.is_empty());
}

#[test]
fn identical_set_never_reinjected_in_same_buffer() {
    let set = || vec![entry(1, Category::Fact, "a"), entry(2, Category::Fact, "b")];

    let mut buf = InjectionBuffer::new();
    buf.set_memories(set(), 1000);
    assert!(buf.take_ready(1001).is_some());
    // Parked after the suppression, then re-taken: the gate is windowless,
    // the set is identical to the last injected one.
    buf.set_memories(set(), 1005);
    assert!(buf.take_ready(1010).is_none());
    // Callers reset by recreating the buffer (jcode's session-scoped state).
    let mut fresh = InjectionBuffer::new();
    fresh.set_memories(set(), 1000);
    assert!(fresh.take_ready(1001).is_some());
}

#[test]
fn disjoint_set_is_not_blocked_by_overlap_gate() {
    let mut buf = InjectionBuffer::new();
    let a = vec![
        entry(1, Category::Fact, "alpha"),
        entry(2, Category::Fact, "beta"),
    ];
    buf.set_memories(a, 1000);
    assert!(buf.take_ready(1001).is_some());

    let disjoint = vec![
        entry(3, Category::Fact, "gamma"),
        entry(4, Category::Fact, "delta"),
    ];
    buf.set_memories(disjoint, 1005);
    assert!(buf.take_ready(1010).is_some());
}

#[test]
fn eighty_percent_overlap_is_blocked_inside_window() {
    let mut buf = InjectionBuffer::new();
    let set = |suffix: &str| {
        vec![
            entry(1, Category::Fact, "a"),
            entry(2, Category::Fact, "b"),
            entry(3, Category::Fact, "c"),
            entry(4, Category::Fact, "d"),
            entry(5, Category::Fact, suffix),
        ]
    };
    buf.set_memories(set("e-old"), 1000);
    assert!(buf.take_ready(1001).is_some());
    // 4 of 5 entries shared -> 80% overlap of the larger set, inside the
    // 180s window; content differs so only the overlap gate can block.
    buf.set_memories(set("e-new"), 1005);
    assert!(buf.take_ready(1010).is_none());
}
