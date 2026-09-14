//! Checkpoint discipline (orbit invariant 4): a failed summary keeps the
//! original messages verbatim — the context survives compaction attempts.

use omenic_harness_compaction::{
    CharBudgetPolicy, CompactionPolicy, Message, NoopSummarizer, Summarizer,
};

fn filler(tag: usize) -> Message {
    Message::user_text(format!("{tag:04}{}", "x".repeat(996)))
}

fn bulk() -> Vec<Message> {
    (0..200).map(filler).collect()
}

/// `None` = backend error / abort (the orbit adapter maps both to this).
struct Failing;
impl Summarizer for Failing {
    fn summarize(&self, _transcript: &str) -> Option<String> {
        None
    }
}

/// A broken summarizer that still "succeeds" with empty text must also be
/// treated as failure — the crate guards it, orbit did.
struct Empty;
impl Summarizer for Empty {
    fn summarize(&self, _transcript: &str) -> Option<String> {
        Some(String::new())
    }
}

#[test]
fn failing_summarizer_keeps_originals() {
    let msgs = bulk();
    let (out, summary) = CharBudgetPolicy::new(Failing).compact_context(None, &msgs, 120_000);
    assert_eq!(out, msgs, "invariant 4: context untouched on failure");
    assert!(
        summary.is_none(),
        "nothing may be logged for a failed summary"
    );
}

#[test]
fn empty_summary_keeps_originals() {
    let msgs = bulk();
    let out = CharBudgetPolicy::new(Empty).compact(&msgs, 120_000);
    assert_eq!(out, msgs);
}

#[test]
fn noop_summarizer_keeps_originals() {
    let msgs = bulk();
    let out = CharBudgetPolicy::new(NoopSummarizer).compact(&msgs, 120_000);
    assert_eq!(out, msgs);
}

/// A run of failed attempts leaves the context bit-stable — compaction
/// retries must not drift the message list.
#[test]
fn retries_are_idempotent_under_failure() {
    let msgs = bulk();
    let policy = CharBudgetPolicy::new(Failing);
    let mut current = msgs.clone();
    for _ in 0..3 {
        current = policy.compact(&current, 120_000);
    }
    assert_eq!(current, msgs);
}
