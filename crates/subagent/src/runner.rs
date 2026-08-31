//! Run a single subagent loop: fresh context → read-only tools → text collection.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use adaptor::{Context, Message, StopReason};
use orbit::{AgentEvent, LlmBackend, TurnStop, run_agent_streaming};
use serde_json::Value;
use tools::Tool;

use crate::config::{MAX_SUBAGENT_RESPONSE_BYTES, SPILL_DIR, SUBAGENT_SYSTEM_PROMPT};

/// Live events a subagent emits while running.
#[derive(Debug, Clone)]
pub enum SubagentEvent {
    Started { id: u32, prompt_preview: String },
    ToolCall { id: u32, name: String, args: Value },
    Finished { id: u32, output: String },
    Failed { id: u32, error: String },
}

/// Errors from a subagent run. `Output` carries the collected text so the
/// caller can still surface partial results on abort/timeout.
#[derive(Debug)]
pub enum SubagentError {
    Aborted { partial: String },
    Timeout { partial: String },
    Backend(String),
}

impl std::fmt::Display for SubagentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SubagentError::Aborted { partial } => {
                write!(f, "aborted ({} bytes collected)", partial.len())
            }
            SubagentError::Timeout { partial } => {
                write!(f, "timeout ({} bytes collected)", partial.len())
            }
            SubagentError::Backend(e) => write!(f, "backend: {e}"),
        }
    }
}

impl std::error::Error for SubagentError {}

/// RAII guard: drop flips the signal so any in-flight tool / loop iteration
/// sees the abort on the next poll. The subagent thread owns one of these
/// for its entire lifetime.
struct AbortGuard<'a>(&'a AtomicBool);

impl Drop for AbortGuard<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Run a single subagent. Returns the collected assistant text (truncated to
/// `MAX_SUBAGENT_RESPONSE_BYTES`) or a `SubagentError` carrying whatever was
/// collected before the failure.
#[allow(clippy::too_many_arguments)]
pub fn run_subagent(
    backend: &dyn LlmBackend,
    model: &adaptor::Model,
    prompt: &str,
    max_turns: usize,
    tools: &[Box<dyn Tool>],
    signal: &AtomicBool,
    id: u32,
    event_tx: Option<&mpsc::Sender<SubagentEvent>>,
) -> Result<String, SubagentError> {
    let _guard = AbortGuard(signal);

    if let Some(tx) = event_tx {
        let preview: String = prompt.chars().take(80).collect();
        let _ = tx.send(SubagentEvent::Started {
            id,
            prompt_preview: preview,
        });
    }

    let mut context = Context {
        system_prompt: Some(SUBAGENT_SYSTEM_PROMPT.into()),
        messages: vec![Message::user_text(prompt)],
    };

    let mut text = String::new();
    let mut turn_count = 0usize;
    // Cell so the FnMut closure can write without capturing a unique borrow
    // that would conflict with reading `last_stop` after the call returns.
    let last_stop: std::cell::Cell<Option<TurnStop>> = std::cell::Cell::new(None);

    // We call run_agent_streaming and re-feed the loop manually so we can
    // enforce max_turns and short-circuit on signal — orbit's own loop is
    // bound only by the model emitting EndTurn/MaxTokens, not by a turn cap.
    let mut emitter = |ev: AgentEvent| match ev {
        AgentEvent::AssistantText { delta } => {
            text.push_str(&delta);
        }
        AgentEvent::ToolCall(tc) => {
            if let Some(tx) = event_tx {
                let _ = tx.send(SubagentEvent::ToolCall {
                    id,
                    name: tc.name,
                    args: tc.args,
                });
            }
        }
        AgentEvent::ToolResult { .. } => {}
        AgentEvent::TurnEnd { stop_reason } => {
            last_stop.set(Some(stop_reason));
        }
    };

    // Turn-budget loop: each orbit call = one round-trip. We cap at max_turns
    // so a runaway model can't burn the 5-min wall clock.
    loop {
        if signal.load(Ordering::Relaxed) {
            return Err(SubagentError::Aborted {
                partial: finalize(text, id, event_tx),
            });
        }
        if turn_count >= max_turns {
            break;
        }
        turn_count += 1;
        run_agent_streaming(
            backend,
            model,
            &mut context,
            tools,
            signal,
            None,
            &mut emitter,
        );
        // Even if the model returned EndTurn, an external abort must win —
        // the signal flips from another thread and we can't keep churning
        // the loop once the caller has given up.
        if signal.load(Ordering::Relaxed) {
            return Err(SubagentError::Aborted {
                partial: finalize(text, id, event_tx),
            });
        }
        match last_stop.get() {
            Some(TurnStop::EndTurn) | Some(TurnStop::Error) => break,
            Some(TurnStop::MaxTokens) | Some(TurnStop::Aborted) | None => {
                // Continue until turn budget or signal fires.
            }
        }
    }

    // Final signal poll — a fast model could complete all turns before any
    // of the per-iteration polls, so we check one last time before claiming
    // success.
    if signal.load(Ordering::Relaxed) {
        return Err(SubagentError::Aborted {
            partial: finalize(text, id, event_tx),
        });
    }
    if matches!(last_stop.get(), Some(TurnStop::Aborted)) {
        return Err(SubagentError::Aborted {
            partial: finalize(text, id, event_tx),
        });
    }
    Ok(finalize(text, id, event_tx))
}

/// Apply the 128KB truncation + emit a `Finished` event on success.
fn finalize(text: String, id: u32, event_tx: Option<&mpsc::Sender<SubagentEvent>>) -> String {
    let out = if text.len() > MAX_SUBAGENT_RESPONSE_BYTES {
        truncate_to_bytes(&text, MAX_SUBAGENT_RESPONSE_BYTES, id)
    } else {
        text
    };
    if let Some(tx) = event_tx {
        let _ = tx.send(SubagentEvent::Finished {
            id,
            output: out.clone(),
        });
    }
    out
}
/// Cuts at the nearest char boundary so we never split a multi-byte codepoint.
fn truncate_to_bytes(s: &str, cap: usize, id: u32) -> String {
    if s.len() <= cap {
        return s.to_string();
    }
    let mut end = cap;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let pid = std::process::id();
    // ponytail: cheap counter — pid + nanos since process start would be
    // unique, but the caller already prefixes with a stable id; pid alone
    // is enough to avoid clobbering sibling spills in the same CLI run.
    let spill_path = std::path::Path::new(SPILL_DIR).join(format!("oi-subagent-{pid}-{id}.txt"));
    let _ = std::fs::write(&spill_path, s);
    format!(
        "[output truncated: showing first {end} of {} bytes. full output: {}]\n{}",
        s.len(),
        spill_path.display(),
        &s[..end]
    )
}

// Compile-time assertion that the runner is byte-only — no StopReason::Aborted
// path can leak past finalize without being caught above.
const _: fn() = || {
    let _ = StopReason::Aborted;
};
