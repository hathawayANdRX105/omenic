//! WP-B end-to-end compaction tests: a >120k-char long session drives the
//! real char-budget policy all the way through the orbit loop (maintain
//! hook → `agent_loop::orbit::compact_context` → harness `compact_with`) with no live
//! LLM and no network.
//!
//! G3 acceptance ② — "`oi` starts one >120k-char long session and the
//! compaction path survives end to end (checkpoint snapshot + keep-original
//! on failure)". Every size below is measured with the policy's own
//! `message_chars`, so the trigger is the genuine context size rather than
//! a hardcoded constant assertion: each test asserts the fixture is really
//! over (or just under) `COMPACT_CHAR_BUDGET` before it asserts anything
//! about the outcome.
//!
//! Setup mirrors `tests/loop.rs`: a `Scripted` backend behind a `RefCell`
//! replays canned turns (the summary stream is just another backend call,
//! since `LlmSummarizer` routes the transcript through the same seam), and
//! `filler`/`bulk` build 1000-char messages so the recent-window cut lands
//! exactly on a message boundary.

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::atomic::AtomicBool;

use agent_loop::compaction::{
    COMPACT_CHAR_BUDGET, KEEP_RECENT_CHARS, KEEP_RECENT_MIN, NoopSummarizer, RegionBudget,
    SUMMARY_PREFIX, compact_with, is_balanced_at, recent_window_cut, to_dto,
};
use agent_loop::orbit::{ContextLog, LlmBackend, message_chars, run_agent, select_compaction_cut};
use llm::{
    Block, Content, Context, Message, Model, StopReason, StreamEvent, ToolCallSpec, ToolDef,
};
use serde_json::json;
use tempfile::tempdir;

fn model() -> Model {
    Model {
        api_key: "k".into(),
        model: "test".into(),
        base_url: None,
        max_tokens: None,
    }
}

fn sig() -> AtomicBool {
    AtomicBool::new(false)
}

/// One tool-call spec, for building assistant messages with `tool_use`
/// blocks in the history (no tool is ever executed here).
fn tc(id: &str) -> ToolCallSpec {
    ToolCallSpec {
        id: id.into(),
        name: "echo_tool".into(),
        args: json!({}),
    }
}

/// A user message of exactly 1000 chars, tagged by index so the retained
/// window and the summarized prefix stay identifiable in the transcript.
fn filler(tag: usize) -> Message {
    Message::user_text(format!("{tag:04}{}", "x".repeat(996)))
}

fn bulk(n: usize) -> Vec<Message> {
    (0..n).map(filler).collect()
}

/// Scripted backend replays canned event lists per call (loop.rs pattern).
struct Scripted {
    turns: Vec<Vec<StreamEvent>>,
    calls_made: usize,
    seen_contexts: Vec<Context>,
}

impl Scripted {
    fn new(turns: Vec<Vec<StreamEvent>>) -> Self {
        Scripted {
            turns,
            calls_made: 0,
            seen_contexts: vec![],
        }
    }
}

/// Trait takes `&self`; tests mutate through the RefCell (loop.rs pattern).
struct Shared(RefCell<Scripted>);
impl LlmBackend for Shared {
    fn stream_cb(
        &self,
        _model: &Model,
        context: &Context,
        _tools: &[ToolDef],
        _signal: &AtomicBool,
        emit: &mut dyn FnMut(&StreamEvent),
    ) {
        let s = &mut *self.0.borrow_mut();
        s.seen_contexts.push(context.clone());
        let t = match s.turns.get(s.calls_made) {
            Some(t) => t.clone(),
            // Turns exhausted: end cleanly so the loop terminates.
            None => vec![StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            }],
        };
        s.calls_made += 1;
        for ev in &t {
            emit(ev);
        }
    }
}

/// Total request size in the policy's own accounting: the system prompt
/// (it rides every call and compaction cannot shrink it) plus messages.
/// This is exactly what `compact_with` measures against the budget.
fn request_chars(ctx: &Context) -> usize {
    let system = ctx.system_prompt.as_deref().map_or(0, str::len);
    system + ctx.messages.iter().map(message_chars).sum::<usize>()
}

/// Re-type wire messages into the harness DTO the policy operates on.
fn dto(msgs: &[Message]) -> Vec<agent_loop::compaction::Message> {
    msgs.iter().map(to_dto).collect()
}

/// (tool_use ids, tool_result ids) carried by one message.
fn tool_ids(m: &Message) -> (Vec<String>, Vec<String>) {
    let Content::Blocks(bs) = &m.content else {
        return (vec![], vec![]);
    };
    let mut uses = Vec::new();
    let mut results = Vec::new();
    for b in bs {
        match b {
            Block::ToolUse { id, .. } => uses.push(id.clone()),
            Block::ToolResult { tool_use_id, .. } => results.push(tool_use_id.clone()),
            _ => {}
        }
    }
    (uses, results)
}

/// Every tool_call id / tool_result id left in a message list.
fn pairing_ids(msgs: &[Message]) -> (HashSet<String>, HashSet<String>) {
    let mut uses = HashSet::new();
    let mut results = HashSet::new();
    for m in msgs {
        let (u, r) = tool_ids(m);
        uses.extend(u);
        results.extend(r);
    }
    (uses, results)
}

/// Orbit invariant 1 / dsh `toolPairingBalancedBefore`: every tool_call in
/// the shipped context carries its matching tool_result and vice versa.
fn pairing_balanced(msgs: &[Message]) -> bool {
    let (uses, results) = pairing_ids(msgs);
    uses == results
}

fn is_summary(m: &Message) -> bool {
    match &m.content {
        Content::Text(s) => s.starts_with(SUMMARY_PREFIX),
        _ => false,
    }
}

/// The transcript the summarizer saw, as plain text.
fn transcript_of(ctx: &Context) -> &str {
    match &ctx.messages[0].content {
        Content::Text(s) => s.as_str(),
        other => panic!("summary request should be plain text: {other:?}"),
    }
}

/// A turn that streams `text` and ends the turn.
fn answer_turn(text: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::TextDelta(text.into()),
        StreamEvent::Done {
            stop_reason: StopReason::EndTurn,
        },
    ]
}

/// A summary stream that produces `gist`, followed by a turn answering `ok`.
fn summary_then_ok(gist: &str) -> Vec<Vec<StreamEvent>> {
    vec![answer_turn(gist), answer_turn("ok")]
}

#[test]
fn long_session_over_budget_triggers_compaction() {
    // 150 messages of exactly 1000 chars = 150,000 chars, well past the
    // 120,000-char trigger. The maintenance hook runs before the first
    // turn, so the summary stream is backend call #1 and the turn is #2.
    let backend = Shared(RefCell::new(Scripted::new(summary_then_ok("the gist"))));
    let mut ctx = Context {
        system_prompt: Some("sys".into()),
        messages: bulk(150),
    };
    assert!(
        request_chars(&ctx) > COMPACT_CHAR_BUDGET,
        "fixture must be a genuinely oversized session"
    );

    let _ = run_agent(&backend, &model(), &mut ctx, &[], &sig(), None);

    let seen = backend.0.borrow();
    // Two backend calls: the summary stream, then the turn itself — no
    // third call, because the compacted context is back under budget.
    assert_eq!(seen.calls_made, 2);
    // The summary request covered the compactable prefix only.
    let transcript = transcript_of(&seen.seen_contexts[0]);
    assert!(
        transcript.contains("0000"),
        "oldest message must be summarized"
    );
    assert!(
        !transcript.contains("0120"),
        "kept window must not be summarized"
    );
    // The turn saw the summary marker plus the verbatim recent window.
    assert_eq!(
        seen.seen_contexts[1].messages.len(),
        1 + KEEP_RECENT_CHARS / 1000
    );
    // The shipped context starts with the marker and is back inside the
    // budget — compaction really cut it, not just rearranged messages.
    assert_eq!(
        ctx.messages[0],
        Message::user_text(format!("{SUMMARY_PREFIX}the gist"))
    );
    assert!(request_chars(&ctx) < COMPACT_CHAR_BUDGET);
}

#[test]
fn compaction_never_splits_tool_call_pairs() {
    // A tool_use message far larger than the recent window pins the raw
    // char-budget cut between the call and its result — exactly the spot
    // that would orphan the result if the pairing guard didn't advance.
    let mut msgs = bulk(100); // idx 0..99: 100,000 chars of history mass
    msgs.push(Message::assistant("A".repeat(35_000), &[tc("t_edge")])); // 100
    msgs.push(Message::tool_results(&[(
        "t_edge".into(),
        "edge result".into(),
    )])); // 101
    msgs.extend(bulk(2)); // 102, 103
    // A complete pair inside the kept window that must survive verbatim.
    msgs.push(Message::assistant("thinking".into(), &[tc("t_keep")])); // 104
    msgs.push(Message::tool_results(&[(
        "t_keep".into(),
        "keep result".into(),
    )])); // 105
    msgs.push(filler(106)); // 106

    let dto_msgs = dto(&msgs);
    let raw_cut = recent_window_cut(&dto_msgs, KEEP_RECENT_CHARS, KEEP_RECENT_MIN);
    assert!(
        !is_balanced_at(&dto_msgs, raw_cut),
        "raw window cut lands between a call and its result"
    );
    let cut = select_compaction_cut(&msgs, KEEP_RECENT_CHARS);
    assert!(
        cut > raw_cut,
        "pairing guard must advance the cut past the orphan"
    );
    assert!(is_balanced_at(&dto_msgs, cut));

    let backend = Shared(RefCell::new(Scripted::new(summary_then_ok("the gist"))));
    let mut ctx = Context {
        system_prompt: Some("sys".into()),
        messages: msgs.clone(),
    };
    assert!(request_chars(&ctx) > COMPACT_CHAR_BUDGET);
    let _ = run_agent(&backend, &model(), &mut ctx, &[], &sig(), None);

    let seen = backend.0.borrow();
    assert_eq!(seen.calls_made, 2);
    // The straddling pair went into the summary TOGETHER: the transcript
    // carries both the call and its result.
    let transcript = transcript_of(&seen.seen_contexts[0]);
    assert!(
        transcript.contains("t_edge"),
        "the orphan call must be summarized"
    );
    assert!(
        transcript.contains("edge result"),
        "its result must be summarized with it, not left orphaned"
    );
    assert!(
        !transcript.contains("t_keep"),
        "the surviving pair must stay verbatim"
    );

    // Shipped context: balanced, with exactly the surviving pair intact.
    assert!(pairing_balanced(&ctx.messages));
    let (uses, results) = pairing_ids(&ctx.messages);
    assert_eq!(uses, results);
    assert_eq!(uses, HashSet::from(["t_keep".to_string()]));
    let kept_use = ctx
        .messages
        .iter()
        .position(|m| tool_ids(m).0 == vec!["t_keep".to_string()])
        .expect("surviving call must stay in the context");
    assert_eq!(
        tool_ids(&ctx.messages[kept_use + 1]).1,
        vec!["t_keep".to_string()],
        "its result must follow immediately"
    );
    assert!(request_chars(&ctx) < COMPACT_CHAR_BUDGET);
}

#[test]
fn failed_summary_keeps_original_and_writes_no_checkpoint() {
    // Policy level first: a summarizer that yields nothing is a pure no-op.
    let dto_msgs = dto(&bulk(150));
    let (out, summary) = compact_with(
        &NoopSummarizer,
        Some("sys"),
        &dto_msgs,
        COMPACT_CHAR_BUDGET,
        &RegionBudget::default(),
    );
    assert!(summary.is_none());
    assert_eq!(out, dto_msgs);

    // End to end: the summary stream errors, so orbit invariant 4 must
    // leave the context and the checkpoint log untouched (no panic).
    let backend = Shared(RefCell::new(Scripted::new(vec![
        vec![StreamEvent::Error("summary backend down".into())],
        answer_turn("ok"),
    ])));
    let initial = bulk(150);
    let mut ctx = Context {
        system_prompt: Some("sys".into()),
        messages: initial.clone(),
    };
    assert!(request_chars(&ctx) > COMPACT_CHAR_BUDGET);
    let dir = tempdir().unwrap();
    let log_path = dir.path().join("ctx.jsonl");
    let log = ContextLog::new(&log_path);
    for m in &initial {
        log.append_message(m).unwrap();
    }

    let _ = run_agent(&backend, &model(), &mut ctx, &[], &sig(), Some(&log));

    // In-memory context: only the final reply appended, no marker injected.
    assert_eq!(ctx.messages.len(), initial.len() + 1);
    assert!(ctx.messages.iter().take(initial.len()).eq(initial.iter()));
    assert!(ctx.messages.iter().all(|m| !is_summary(m)));
    // The turn ran against every original message — nothing was dropped.
    let seen = backend.0.borrow();
    assert_eq!(seen.calls_made, 2);
    assert_eq!(seen.seen_contexts[1].messages, initial);
    // Checkpoint snapshot: the replayed log is the pre-compaction
    // conversation followed by the reply — no compaction marker anywhere.
    let replayed = ContextLog::load(&log_path).unwrap();
    assert_eq!(replayed.len(), initial.len() + 1);
    assert!(replayed.starts_with(initial.as_slice()));
    assert_eq!(replayed[initial.len()], ctx.messages[initial.len()]);
}

#[test]
fn session_just_under_budget_triggers_no_compaction() {
    // Exactly one character under the trigger: the boundary itself is
    // probed, not a comfortably small context. The system prompt counts
    // toward the budget, so it carries the remaining slack.
    let messages = bulk(119); // 119,000 chars
    let under = COMPACT_CHAR_BUDGET.saturating_sub(1);
    assert!(
        under > 119_000,
        "fixture assumes the char budget exceeds 119k characters"
    );
    let mut ctx = Context {
        system_prompt: Some("s".repeat(under - 119_000)),
        messages,
    };
    assert_eq!(request_chars(&ctx), COMPACT_CHAR_BUDGET - 1);
    let before = ctx.messages.clone();

    let backend = Shared(RefCell::new(Scripted::new(vec![answer_turn("ok")])));
    let _ = run_agent(&backend, &model(), &mut ctx, &[], &sig(), None);

    // Exactly one backend call — the turn itself. No summary stream.
    let seen = backend.0.borrow();
    assert_eq!(seen.calls_made, 1);
    assert_eq!(seen.seen_contexts[0].messages, before);
    // Originals untouched; only the final reply was appended.
    assert_eq!(ctx.messages.len(), before.len() + 1);
    assert!(ctx.messages.iter().take(before.len()).eq(before.iter()));
    assert!(ctx.messages.iter().all(|m| !is_summary(m)));
}
