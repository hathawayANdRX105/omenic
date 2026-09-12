//! End-to-end coverage for the memory wiring (orbit → tools → store →
//! recall, plus the injection seam and the extraction boundary). Every
//! store lives in a tempdir behind the real env gate; the LLM is a
//! scripted backend so nothing here touches a network.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use memory::pipeline::extract_lines;
use memory::MemoryEntry;
use orbit::LlmBackend;
use serde_json::json;
use tools::Tool;

use adaptor::StreamEvent::{Done, ToolCall};
use adaptor::*;

fn mock_model() -> Model {
    Model {
        api_key: "k".into(),
        model: "mock".into(),
        base_url: None,
        max_tokens: None,
    }
}

fn sig() -> AtomicBool {
    AtomicBool::new(false)
}

/// Scripted backend: one canned event list per call (consumed in order),
/// recording every context it is handed.
struct Scripted {
    turns: Mutex<Vec<Vec<StreamEvent>>>,
    seen: Mutex<Vec<Context>>,
}

impl Scripted {
    fn new(turns: Vec<Vec<StreamEvent>>) -> Self {
        Scripted {
            turns: Mutex::new(turns),
            seen: Mutex::new(Vec::new()),
        }
    }
}

impl LlmBackend for Scripted {
    fn stream_cb(
        &self,
        _model: &Model,
        context: &Context,
        _tools: &[adaptor::ToolDef],
        _signal: &AtomicBool,
        emit: &mut dyn FnMut(&StreamEvent),
    ) {
        self.seen.lock().unwrap().push(context.clone());
        let mut queue = self.turns.lock().unwrap();
        if queue.is_empty() {
            drop(queue);
            emit(&Done {
                stop_reason: StopReason::EndTurn,
            });
            return;
        }
        let next = queue.remove(0);
        drop(queue);
        for ev in &next {
            emit(ev);
        }
    }
}

// ===== env gate =====

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Point the memory gate at `dir` for the duration of `body`, restoring the
/// unset state afterwards (see memory_tool tests for the rationale).
fn with_store<R>(dir: &std::path::Path, body: impl FnOnce() -> R) -> R {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    // SAFETY: every env-mutating test in this binary holds ENV_LOCK.
    unsafe {
        std::env::set_var("OMENIC_MEMORY", "1");
        std::env::set_var("OMENIC_MEMORY_DIR", dir);
    }
    let out = body();
    unsafe {
        std::env::remove_var("OMENIC_MEMORY");
        std::env::remove_var("OMENIC_MEMORY_DIR");
    }
    out
}

fn call(tool: &dyn Tool, args: &serde_json::Value) -> String {
    tool.execute(args, &sig()).unwrap()
}

// ===== tests =====

/// The whole chain: an agent run whose model emits a `memory_append` tool
/// call must land the row in the real store, and a follow-up run using
/// `memory_search` must recall it (loop → tools → store → recall).
#[test]
fn agent_run_appends_and_searches_real_store() {
    let dir = tempfile::tempdir().unwrap();
    with_store(dir.path(), || {
        // First run: the model asks for the tool, orbit executes it for real.
        let backend = Scripted::new(vec![
            vec![
                ToolCall(ToolCallSpec {
                    id: "m1".into(),
                    name: "memory_append".into(),
                    args: json!({"text": "deploy target is fly.io", "category": "fact"}),
                }),
                Done {
                    stop_reason: StopReason::ToolUse,
                },
            ],
            vec![
                adaptor::StreamEvent::TextDelta("noted".into()),
                Done {
                    stop_reason: StopReason::EndTurn,
                },
            ],
        ]);
        let events = orbit::run_agent(
            &backend,
            &mock_model(),
            &mut Context::default(),
            &tools::builtin_tools(),
            &sig(),
            None,
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, orbit::AgentEvent::ToolResult { result, .. }
                    if result.starts_with("remembered #"))),
            "tool must run against the real store: {events:?}"
        );
        assert_eq!(tools::memory_tool::memory_store().list().unwrap().len(), 1);

        // Second run: search recalls it through the same chain.
        let backend = Scripted::new(vec![
            vec![
                ToolCall(ToolCallSpec {
                    id: "m2".into(),
                    name: "memory_search".into(),
                    args: json!({"query": "deploy"}),
                }),
                Done {
                    stop_reason: StopReason::ToolUse,
                },
            ],
            vec![
                adaptor::StreamEvent::TextDelta("done".into()),
                Done {
                    stop_reason: StopReason::EndTurn,
                },
            ],
        ]);
        let events = orbit::run_agent(
            &backend,
            &mock_model(),
            &mut Context::default(),
            &tools::builtin_tools(),
            &sig(),
            None,
        );
        let search_result = events
            .iter()
            .find_map(|e| match e {
                orbit::AgentEvent::ToolResult { result, .. } => Some(result.clone()),
                _ => None,
            })
            .unwrap();
        assert!(
            search_result.contains("1 hits:") && search_result.contains("fly.io"),
            "search must surface the stored row: {search_result}"
        );
    });
}

/// The injection seam: a queued payload drains into the context as a
/// trailing user message before the next model call sees it, exactly once.
#[test]
fn queued_injection_reaches_the_next_call() {
    let dir = tempfile::tempdir().unwrap();
    with_store(dir.path(), || {
        let mut entry = MemoryEntry::new("reinforce: ci runs on monday");
        entry.id = 1;
        entry.ts = "2026-01-01T00:00:00Z".into();
        entry.category = memory::Category::Correction;
        web::memory_link::set_injection("sess-1", vec![entry], 1000);

        let mut context = Context::default();
        context.messages.push(Message::user_text("hi"));
        assert!(web::memory_link::drain_injection(
            "sess-1",
            &mut context,
            1050
        ));

        let backend = Scripted::new(vec![]);
        let _ = orbit::run_agent(&backend, &mock_model(), &mut context, &[], &sig(), None);
        let seen = backend.seen.lock().unwrap();
        let last = seen[0].messages.last().unwrap();
        match &last.content {
            Content::Text(t) => {
                assert!(t.contains("<system-reminder>"), "payload: {t}");
                assert!(t.contains("ci runs on monday"), "payload: {t}");
            }
            other => panic!("expected plain text reminder, got {other:?}"),
        }
        // Drained once: a second call finds nothing.
        let mut again = Context::default();
        assert!(!web::memory_link::drain_injection(
            "sess-1", &mut again, 1100
        ));
    });
}

/// The extraction boundary: the extractor output is parsed and persisted
/// when the trigger fires; an empty extractor output stores nothing, and a
/// non-firing trigger does not reach the extractor at all.
#[test]
fn extraction_stores_lines_only_after_trigger() {
    let dir = tempfile::tempdir().unwrap();
    with_store(dir.path(), || {
        let transcript = vec![
            Message::user_text("we moved deploy to fly.io, staging is fly-east"),
            Message::assistant_text("understood, noted"),
        ];
        let extractor_answer = || {
            vec![vec![
                adaptor::StreamEvent::TextDelta(
                    "fact|deploy target is fly.io|high\ncorrection|端口 8026 不是 3000|medium\n"
                        .into(),
                ),
                Done {
                    stop_reason: StopReason::EndTurn,
                },
            ]]
        };

        // First call on a fresh session: the trigger fires (first ever).
        let extractor = Scripted::new(extractor_answer());
        let stored = web::memory_link::extract_and_remember(
            &extractor,
            &mock_model(),
            "s-1",
            1,
            false,
            &transcript,
            1000,
        );
        assert_eq!(stored, 2, "both lines land: {stored}");
        let rows = tools::memory_tool::memory_store().list().unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|r| r.text.contains("fly.io")));
        assert!(rows.iter().any(|r| r.text.contains("8026")));

        // Second call, same session, nothing special queued: the trigger
        // does not fire, the extractor is never consulted.
        let extractor = Scripted::new(vec![]);
        let stored = web::memory_link::extract_and_remember(
            &extractor,
            &mock_model(),
            "s-1",
            2,
            false,
            &transcript,
            1100,
        );
        assert_eq!(stored, 0);
        assert!(
            extractor.seen.lock().unwrap().is_empty(),
            "extractor must not run when the trigger does not fire"
        );

        // Session-end forces the trigger even without turn cadence.
        let extractor = Scripted::new(extractor_answer());
        let stored = web::memory_link::extract_and_remember(
            &extractor,
            &mock_model(),
            "s-1",
            3,
            true,
            &transcript,
            1200,
        );
        assert_eq!(stored, 2);
        assert_eq!(extractor.seen.lock().unwrap().len(), 1);

        // Empty extractor output stores nothing.
        let extractor = Scripted::new(vec![vec![Done {
            stop_reason: StopReason::EndTurn,
        }]]);
        let stored = web::memory_link::extract_and_remember(
            &extractor,
            &mock_model(),
            "s-2",
            1,
            true,
            &transcript,
            1300,
        );
        assert_eq!(stored, 0);

        // Sanity: the parser used by the wiring round-trips the fixtures.
        assert_eq!(extract_lines("fact|x|high").len(), 1);
    });
}
