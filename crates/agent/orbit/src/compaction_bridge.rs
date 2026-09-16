//! Compaction call seam: the `LlmBackend` <-> harness `Summarizer` bridge.
//!
//! Policy (char-budget window, tool-pairing invariant, kept-window guard,
//! transcript rendering) lives in `omenic-harness-compaction` (C4). What
//! remains here is the host bridge only: the LLM-backed summarizer over the
//! loop's own backend, the wire<->DTO re-typing, and the maintenance hook a
//! host installs as [`crate::LoopConfig::maintain`]. Zero policy logic — the
//! trigger budget and the verbatim-window region come from the
//! [`CharBudgetPolicy`] the host resolved out of the assembled container, so
//! `&CharBudgetPolicy::default()` reproduces the historic hardcoded
//! `COMPACT_CHAR_BUDGET` + default region byte for byte.
//!
//! Lives in the agent domain (not in `crates/harness/compaction`) on purpose:
//! the bridge speaks `adaptor` streaming types and orbit's own
//! [`LlmBackend`] / [`crate::ContextLog`], and a harness crate may not depend
//! on the agent domain — while orbit depending on a bridge crate that depends
//! on orbit would be a cycle. Isolating the seam in this module keeps
//! `lib.rs` under the R3 <= 20 line budget.

use std::sync::atomic::{AtomicBool, Ordering};

use adaptor::{Context, Message, Model, StopReason, StreamEvent};
use omenic_harness_compaction::{
    CharBudgetPolicy, Summarizer, compact_with, message_chars as dto_chars,
    select_compaction_cut as dto_cut, to_dto, to_wire,
};

use crate::{ContextLog, LlmBackend};

/// Estimated size of one message in characters (delegates to the policy crate).
pub fn message_chars(m: &Message) -> usize {
    dto_chars(&to_dto(m))
}

/// First index to keep verbatim under a recent-window char budget.
pub fn select_compaction_cut(messages: &[Message], budget: usize) -> usize {
    dto_cut(&messages.iter().map(to_dto).collect::<Vec<_>>(), budget)
}

/// Bridges the harness `Summarizer` hook onto orbit's [`LlmBackend`]: the
/// summary stream runs against the same scripted/HTTP backend as the loop.
pub struct LlmSummarizer<'a>(&'a dyn LlmBackend, &'a Model, &'a AtomicBool);

impl Summarizer for LlmSummarizer<'_> {
    fn summarize(&self, transcript: &str) -> Option<String> {
        let ctx = Context {
            system_prompt: Some(
                "请将以下对话总结为简洁的上下文摘要，保留关键决策、已做的工作和待办事项。".into(),
            ),
            messages: vec![Message::user_text(transcript)],
        };
        let mut summary = String::new();
        for ev in self.0.stream(self.1, &ctx, &[], self.2) {
            match ev {
                StreamEvent::TextDelta(delta) => summary.push_str(&delta),
                StreamEvent::Done {
                    stop_reason: StopReason::Aborted,
                }
                | StreamEvent::Error(_) => return None,
                _ => {}
            }
        }
        (!summary.is_empty()).then_some(summary)
    }
}

/// The default host maintenance hook for [`crate::LoopConfig::maintain`];
/// hosts may substitute their own. Invariant 4: on any failure the context
/// is left untouched.
///
/// Equivalent to [`compact_context_with`] on [`CharBudgetPolicy::default`]:
/// the historic hardcoded `COMPACT_CHAR_BUDGET` + default region. Kept as the
/// 5-arg entry point so a host that has no resolved policy (and the orbit
/// test suite) gets the documented default behavior.
///
/// Traceability (EC-7): the injected summary marker goes through
/// `context_log` too, so a replay of the log shows the full pre-compaction
/// conversation followed by the marker in its actual position.
pub fn compact_context(
    backend: &dyn LlmBackend,
    model: &Model,
    context: &mut Context,
    signal: &AtomicBool,
    context_log: Option<&ContextLog>,
) {
    compact_context_with(
        backend,
        model,
        context,
        signal,
        context_log,
        &CharBudgetPolicy::default(),
    );
}

/// The container-driven seam: same maintenance pass, but the trigger budget
/// and the verbatim-window region come from a `CharBudgetPolicy` the host
/// resolved out of the assembled plugin container (`harness.compaction`).
/// The summary itself always streams through the loop's own backend, so a
/// policy resolved with the keep-original default summarizer still compacts
/// for real here — the policy decides *when* the context is rewritten, not
/// how the summary is produced.
pub fn compact_context_with(
    backend: &dyn LlmBackend,
    model: &Model,
    context: &mut Context,
    signal: &AtomicBool,
    context_log: Option<&ContextLog>,
    policy: &CharBudgetPolicy,
) {
    if signal.load(Ordering::Relaxed) {
        return;
    }
    let dto: Vec<_> = context.messages.iter().map(to_dto).collect();
    let (out, summary) = compact_with(
        &LlmSummarizer(backend, model, signal),
        context.system_prompt.as_deref(),
        &dto,
        policy.total_chars(),
        &policy.region(),
    );
    context.messages = out.iter().map(to_wire).collect();
    if let (Some(log), Some(msg)) = (context_log, &summary) {
        let _ = log.append(&to_wire(msg));
    }
}
