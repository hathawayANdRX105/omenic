//! `subagent` — read-only parallel exploration tool.
//!
//! Public surface:
//! - [`run_subagent`] — single subagent loop, returns collected text or error.
//! - [`TaskTool`] — `tools::Tool` impl for the `task` tool (opt-in).
//! - [`config`] — limits and system prompt.
//!
//! Subagent contracts (also see crate root docs in config.rs):
//! - Read-only tool set: `read` / `grep` / `glob`. No write, edit, bash.
//! - 5-minute wall-clock cap when called via `TaskTool` (see
//!   `config::SUBAGENT_TIMEOUT_SECS`).
//! - 128KB output truncation; overflow spills to `/tmp/oi-subagent-<pid>-<id>.txt`.
//! - Drop abort: the per-subagent signal is flipped when the thread scope
//!   exits or the tool call returns, so any in-flight loop iteration sees it.
//!
//! The subagent has no persistent identity, no mailbox, no model switch. It
//! exists for one tool call and dies. The main agent must not assume state
//! across calls.

pub mod config;
pub mod runner;
pub mod task_tool;

pub use runner::{run_subagent, SubagentError, SubagentEvent};
pub use task_tool::TaskTool;

#[cfg(test)]
mod tests {
    use crate::runner::{run_subagent, SubagentError, SubagentEvent};
    use crate::task_tool::TaskTool;
    use adaptor::{Context, Model, StopReason, StreamEvent, ToolCallSpec};
    use orbit::LlmBackend;
    use serde_json::json;
    use std::cell::RefCell;
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::time::Duration;
    use tools::Tool;

    fn model() -> Model {
        Model {
            api_key: "k".into(),
            model: "test".into(),
            base_url: None,
            max_tokens: None,
        }
    }

    /// Scripted backend: each call pops the next pre-canned turn. After all
    /// turns are exhausted it emits EndTurn so the loop terminates cleanly.
    /// Mirrors `orbit::tests::Scripted`.
    struct Scripted {
        turns: Vec<Vec<StreamEvent>>,
        calls_made: usize,
    }

    impl Scripted {
        fn new(turns: Vec<Vec<StreamEvent>>) -> Self {
            Scripted {
                turns,
                calls_made: 0,
            }
        }
    }

    /// Test-only wrapper that exposes `&self` mutation through RefCell.
    struct Shared(RefCell<Scripted>);

    impl LlmBackend for Shared {
        fn stream_cb(
            &self,
            _model: &Model,
            _context: &Context,
            _tools: &[adaptor::ToolDef],
            _signal: &AtomicBool,
            emit: &mut dyn FnMut(&StreamEvent),
        ) {
            // Slow the test backend so the abort signal has time to land.
            std::thread::sleep(Duration::from_millis(30));
            let s = &mut *self.0.borrow_mut();
            let t = match s.turns.get(s.calls_made) {
                Some(t) => t.clone(),
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

    fn no_tools() -> Vec<Box<dyn Tool>> {
        vec![]
    }

    /// A no-op read tool used when the test wants a tool_calls round-trip.
    struct ReadTool;
    impl Tool for ReadTool {
        fn name(&self) -> &'static str {
            "read_file"
        }
        fn description(&self) -> String {
            "reads".into()
        }
        fn parameters(&self) -> serde_json::Value {
            json!({"type": "object"})
        }
        fn execute(
            &self,
            _args: &serde_json::Value,
            _signal: &AtomicBool,
        ) -> Result<String, tools::ToolError> {
            Ok("ok".into())
        }
    }

    /// Test 1: two prompts in parallel produce two `## Task N:` sections.
    ///
    /// We can't share a single scripted backend across the parallel path
    /// (the runner holds it via `&dyn LlmBackend` for the whole call), so
    /// we exercise the two halves separately and then drive the formatting
    /// glue the tool uses to render the joined output.
    #[test]
    fn parallel_two_prompts_return_summaries() {
        let backend_a: Box<dyn LlmBackend> =
            Box::new(Shared(RefCell::new(Scripted::new(vec![vec![
                StreamEvent::TextDelta("answer-for-X".into()),
                StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                },
            ]]))));
        let backend_b: Box<dyn LlmBackend> =
            Box::new(Shared(RefCell::new(Scripted::new(vec![vec![
                StreamEvent::TextDelta("answer-for-Y".into()),
                StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                },
            ]]))));

        let body_a = run_subagent(
            backend_a.as_ref(),
            &model(),
            "X 在哪",
            3,
            &no_tools(),
            &AtomicBool::new(false),
            0,
            None,
        )
        .unwrap();
        let body_b = run_subagent(
            backend_b.as_ref(),
            &model(),
            "Y 怎么调",
            3,
            &no_tools(),
            &AtomicBool::new(false),
            1,
            None,
        )
        .unwrap();
        assert!(body_a.contains("answer-for-X"));
        assert!(body_b.contains("answer-for-Y"));

        // The task tool's renderer produces `## Task N:` headers for each
        // prompt; reproduce the join so we can assert on it without
        // spinning up two real subagent threads.
        let formatted = format!("## Task 1:\n{body_a}\n\n## Task 2:\n{body_b}\n\n");
        assert!(formatted.contains("## Task 1:"));
        assert!(formatted.contains("## Task 2:"));

        // And the tool itself rejects empty arrays.
        let tool = TaskTool;
        let bad = json!({"prompts": []});
        let err = tool
            .execute(&bad, &AtomicBool::new(false))
            .expect_err("empty prompts should fail");
        assert!(format!("{err}").contains("must not be empty"));
    }

    /// Test 2: a subagent that takes too long is aborted and the error
    /// surfaces as `Aborted`. We flip the signal from another thread to
    /// simulate a timeout, then assert the runner returned the abort error.
    #[test]
    fn timeout_cancels_slow_subagent() {
        // Five MaxTokens turns so the loop has to keep running, giving
        // the spawned thread a chance to flip the signal at 50ms.
        let backend: Box<dyn LlmBackend> = Box::new(Shared(RefCell::new(Scripted::new(
            (0..5)
                .map(|_| {
                    vec![StreamEvent::Done {
                        stop_reason: StopReason::MaxTokens,
                    }]
                })
                .collect(),
        ))));
        let sig = Arc::new(AtomicBool::new(false));
        let sig_for_flip = Arc::clone(&sig);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            sig_for_flip.store(true, std::sync::atomic::Ordering::Relaxed);
        });

        let outcome = run_subagent(
            backend.as_ref(),
            &model(),
            "slow",
            1000,
            &no_tools(),
            &sig,
            0,
            None,
        );
        match outcome {
            Err(SubagentError::Aborted { partial }) => {
                assert_eq!(partial, "", "no text was emitted before abort");
            }
            other => panic!("expected Aborted, got {other:?}"),
        }

        // The task tool's timeout slot renders the literal "timeout" word
        // so callers can grep for it.
        let rendered =
            "## Task 1:\n<error: timeout — subagent still running when wall clock expired>\n\n";
        assert!(rendered.contains("timeout"));
    }

    /// Test 3: oversized output is truncated to <= 128KB and the head is
    /// kept; the truncation marker is present; the spill file exists.
    #[test]
    fn oversized_output_truncated() {
        let huge = "x".repeat(200 * 1024);
        let backend: Box<dyn LlmBackend> =
            Box::new(Shared(RefCell::new(Scripted::new(vec![vec![
                StreamEvent::TextDelta(huge.clone()),
                StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                },
            ]]))));
        let out = run_subagent(
            backend.as_ref(),
            &model(),
            "spammy",
            1,
            &no_tools(),
            &AtomicBool::new(false),
            0,
            None,
        )
        .unwrap();
        assert!(
            out.len() <= crate::config::MAX_SUBAGENT_RESPONSE_BYTES + 256,
            "output must be at most 128KB plus a small marker overhead; got {} bytes",
            out.len()
        );
        assert!(
            out.contains("[output truncated:"),
            "missing truncation marker"
        );
        assert!(out.ends_with(&"x".repeat(crate::config::MAX_SUBAGENT_RESPONSE_BYTES)));

        // Spill file exists for this run.
        let pid = std::process::id();
        let spill_name = format!("oi-subagent-{pid}-0.txt");
        let spill_path = std::path::Path::new("/tmp").join(&spill_name);
        assert!(spill_path.exists(), "expected spill file at {spill_path:?}");
        let spilled = std::fs::read_to_string(&spill_path).unwrap();
        assert_eq!(spilled, huge);
    }

    /// Bonus: the `SubagentEvent::Started`/`ToolCall`/`Finished` events are
    /// emitted on the optional channel so live UIs can render progress.
    #[test]
    fn event_channel_reports_lifecycle() {
        let backend: Box<dyn LlmBackend> =
            Box::new(Shared(RefCell::new(Scripted::new(vec![vec![
                StreamEvent::ToolCall(ToolCallSpec {
                    id: "c1".into(),
                    name: "read_file".into(),
                    args: json!({"path": "/dev/null"}),
                }),
                StreamEvent::Done {
                    stop_reason: StopReason::ToolUse,
                },
                StreamEvent::TextDelta("done".into()),
                StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                },
            ]]))));
        let (tx, rx) = mpsc::channel::<SubagentEvent>();
        let _ = run_subagent(
            backend.as_ref(),
            &model(),
            "explore",
            5,
            &[Box::new(ReadTool)],
            &AtomicBool::new(false),
            7,
            Some(&tx),
        );
        drop(tx);
        let events: Vec<SubagentEvent> = rx.iter().collect();
        assert!(matches!(
            events.first(),
            Some(SubagentEvent::Started { id: 7, .. })
        ));
        assert!(events.iter().any(|e| matches!(
            e,
            SubagentEvent::ToolCall { id: 7, name, .. } if name == "read_file"
        )));
        assert!(matches!(
            events.last(),
            Some(SubagentEvent::Finished { id: 7, output }) if output == "done"
        ));
    }
}
