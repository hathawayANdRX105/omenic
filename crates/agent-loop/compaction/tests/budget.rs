//! Char-budget boundaries of `CharBudgetPolicy` — the orbit 120k policy
//! ported verbatim, exercised with exact-arithmetic 1000-char messages.

use compaction::{CharBudgetPolicy, CompactionPolicy, KEEP_RECENT_CHARS, Message, NoopSummarizer};

fn filler(tag: usize) -> Message {
    Message::user_text(format!("{tag:04}{}", "x".repeat(996)))
}

fn bulk(n: usize) -> Vec<Message> {
    (0..n).map(filler).collect()
}

/// Always-success stub: every requested summary becomes `"the gist"`.
struct Always(&'static str);
impl compaction::Summarizer for Always {
    fn summarize(&self, _transcript: &str) -> Option<String> {
        Some(self.0.to_string())
    }
}

/// 200 × 1000 = 200_000 chars.
const TOTAL: usize = 200_000;

/// Exactly at the budget: compaction triggers (the port keeps orbit's
/// `<` semantics — reaching the budget is over the line).
#[test]
fn budget_exactly_equal_compacts() {
    let msgs = bulk(TOTAL / 1000);
    let policy = CharBudgetPolicy::new(Always("the gist"));
    let out = policy.compact(&msgs, TOTAL);
    assert_eq!(out[0], Message::user_text("[context summary]\nthe gist"));
    // newest KEEP_RECENT_CHARS window survives verbatim.
    let kept = KEEP_RECENT_CHARS / 1000;
    assert_eq!(out.len(), 1 + kept);
    assert_eq!(&out[1..], &msgs[msgs.len() - kept..]);
}

/// Over the budget by a single char: same trigger, same window.
#[test]
fn budget_over_by_one_compacts() {
    let msgs = bulk(TOTAL / 1000);
    let out = CharBudgetPolicy::new(Always("g")).compact(&msgs, TOTAL - 1);
    assert_eq!(out.len(), 1 + KEEP_RECENT_CHARS / 1000);
    assert_eq!(
        out[0].content,
        Message::user_text("[context summary]\ng").content
    );
}

/// One char below the budget: nothing happens, byte-identical clone.
#[test]
fn budget_below_keeps_everything() {
    let msgs = bulk(TOTAL / 1000);
    let out = CharBudgetPolicy::new(Always("g")).compact(&msgs, TOTAL + 1);
    assert_eq!(out, msgs);
}

/// Under budget with a system prompt counted in: the prompt's chars join
/// the accounting (orbit's `context_chars` semantics).
#[test]
fn system_prompt_counts_toward_the_budget() {
    let msgs = bulk(10); // 10_000 chars
    let policy = CharBudgetPolicy::new(Always("g"));
    let (out, summary) = policy.compact_context(Some(&"s".repeat(190_000)), &msgs, TOTAL);
    assert_eq!(
        out, msgs,
        "190_000 + 10_000 reaches the budget but the 10k window fits"
    );
    assert!(summary.is_none());
}

/// Whole context smaller than the recent window: `cut == 0`, no-op even
/// when over total budget (nothing older than the window to summarize).
#[test]
fn nothing_older_than_window_is_kept() {
    let msgs = bulk(20); // 20_000 < KEEP_RECENT_CHARS
    let out = CharBudgetPolicy::new(Always("g")).compact(&msgs, 1);
    assert_eq!(out, msgs);
}

/// The kept window alone meets/exceeds the budget (giant newest messages):
/// summarizing the prefix cannot help — keep original (orbit's guard, and
/// the reason an over-budget floor is not a compaction request).
#[test]
fn kept_window_alone_over_budget_is_kept() {
    let mut msgs = bulk(5);
    msgs.push(Message::user_text("A".repeat(200_000)));
    msgs.push(Message::user_text("B".repeat(200_000)));
    let total: usize = msgs.iter().map(compaction::message_chars).sum();
    // Budget at the floor-window size: the guard, not `cut == 0`, stops this.
    let out = CharBudgetPolicy::new(Always("g")).compact(&msgs, total - 5_000);
    assert_eq!(out, msgs);
}

#[test]
fn degenerate_contexts_are_no_ops() {
    let p = CharBudgetPolicy::new(Always("g"));
    assert_eq!(p.compact(&[], 0), Vec::<Message>::new());
    let one = vec![filler(0)];
    assert_eq!(p.compact(&one, 0), one);
}

/// The default service instance is the safe one: no summarizer wired means
/// compaction never rewrites.
#[test]
fn noop_policy_is_identity() {
    let msgs = bulk(TOTAL / 1000);
    let out = CharBudgetPolicy::new(NoopSummarizer).compact(&msgs, 1);
    assert_eq!(out, msgs);
}

/// The trait object speaks the same policy as the concrete type.
#[test]
fn trait_object_compacts_like_concrete() {
    let msgs = bulk(TOTAL / 1000);
    let concrete = CharBudgetPolicy::new(Always("g"));
    let dyn_ref: &dyn CompactionPolicy = &concrete;
    assert_eq!(
        dyn_ref.compact(&msgs, TOTAL),
        concrete.compact(&msgs, TOTAL)
    );
}
