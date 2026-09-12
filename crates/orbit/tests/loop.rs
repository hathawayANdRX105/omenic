//! Agent-loop invariant tests, driven by a scripted backend so every
//! stream/abort/truncation path is exercised offline.
//!
//! Compaction policy units (`select_compaction_cut`) live here too — the
//! loop itself only sees the host-provided `maintain` hook.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

use adaptor::{Context, Message, Model, StopReason, StreamEvent, ToolCallSpec, ToolDef};
use orbit::{
    AgentEvent, ContextLog, KEEP_RECENT_CHARS, KEEP_RECENT_MIN, LlmBackend, LoopConfig, TurnStop,
    message_chars, run_agent, run_agent_streaming, select_compaction_cut,
};
use serde_json::json;
use tools::Tool;

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

fn call(id: &str, name: &str) -> StreamEvent {
    StreamEvent::ToolCall(ToolCallSpec {
        id: id.into(),
        name: name.into(),
        args: json!({}),
    })
}

struct EchoTool;

impl Tool for EchoTool {
    fn name(&self) -> &'static str {
        "echo_tool"
    }
    fn description(&self) -> String {
        "echoes".into()
    }
    fn parameters(&self) -> serde_json::Value {
        json!({"type": "object"})
    }
    fn execute(
        &self,
        _args: &serde_json::Value,
        signal: &AtomicBool,
    ) -> Result<String, tools::ToolError> {
        if signal.load(Ordering::Relaxed) {
            return Ok("aborted".into());
        }
        Ok("echo!".into())
    }
}

/// Scripted backend replays canned event lists per call.
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

/// Trait takes `&self`; tests mutate through RefCell.
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

#[test]
fn plain_answer_ends_loop() {
    let backend = Shared(RefCell::new(Scripted::new(vec![vec![
        StreamEvent::TextDelta("hello ".into()),
        StreamEvent::TextDelta("world".into()),
        StreamEvent::Done {
            stop_reason: StopReason::EndTurn,
        },
    ]])));
    let mut ctx = Context::default();
    ctx.system_prompt = Some("sys".into());
    let events = run_agent(&backend, &model(), &mut ctx, &[], &sig(), None);

    assert_eq!(
        events,
        vec![
            AgentEvent::TurnStart,
            AgentEvent::AssistantText {
                delta: "hello ".into()
            },
            AgentEvent::AssistantText {
                delta: "world".into()
            },
            AgentEvent::TurnEnd {
                stop_reason: TurnStop::EndTurn
            },
        ]
    );
    assert_eq!(ctx.messages.len(), 1);
    assert_eq!(
        ctx.messages[0],
        Message::assistant("hello world".into(), &[])
    );
}

#[test]
fn tool_call_executes_and_result_is_backfilled() {
    let backend = Shared(RefCell::new(Scripted::new(vec![
        vec![
            call("t1", "echo_tool"),
            StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
            },
        ],
        vec![
            StreamEvent::TextDelta("done".into()),
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            },
        ],
    ])));
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];
    let mut ctx = Context::default();
    let events = run_agent(&backend, &model(), &mut ctx, &tools, &sig(), None);

    assert!(events.contains(&AgentEvent::ToolResult {
        id: "t1".into(),
        name: "echo_tool".into(),
        result: "echo!".into(),
    }));
    // assistant(tool_use) then user(tool_result) then assistant(final).
    assert_eq!(ctx.messages.len(), 3);
    assert_eq!(
        ctx.messages[1],
        Message::tool_results(&[("t1".into(), "echo!".into())])
    );
}

#[test]
fn invariant_2_max_tokens_truncation_skips_execution() {
    let backend = Shared(RefCell::new(Scripted::new(vec![
        vec![
            call("t1", "echo_tool"),
            StreamEvent::Done {
                stop_reason: StopReason::MaxTokens,
            },
        ],
        // Retry turn succeeds.
        vec![StreamEvent::Done {
            stop_reason: StopReason::EndTurn,
        }],
    ])));
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];
    let mut ctx = Context::default();
    let events = run_agent(&backend, &model(), &mut ctx, &tools, &sig(), None);

    assert!(
        events.contains(&AgentEvent::ToolResult {
            id: "t1".into(),
            name: "echo_tool".into(),
            result:
                "error: output truncated by max_tokens, tool \"echo_tool\" args may be incomplete."
                    .into(),
        })
    );
    // The tool never executed; the retry saw exactly one backfilled user message.
    assert_eq!(ctx.messages.len(), 3);
    assert_eq!(
        ctx.messages[1],
        Message::tool_results(&[(
            "t1".into(),
            "error: output truncated by max_tokens, tool \"echo_tool\" args may be incomplete."
                .into()
        )])
    );
}

#[test]
fn invariant_3_abort_drops_pending_tool_calls() {
    let backend = Shared(RefCell::new(Scripted::new(vec![vec![
        StreamEvent::TextDelta("partial".into()),
        call("t1", "echo_tool"),
        StreamEvent::Done {
            stop_reason: StopReason::Aborted,
        },
    ]])));
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];
    let mut ctx = Context::default();
    let events = run_agent(&backend, &model(), &mut ctx, &tools, &sig(), None);

    assert_eq!(
        events.last(),
        Some(&AgentEvent::TurnEnd {
            stop_reason: TurnStop::Aborted
        })
    );
    // Assistant message recorded WITHOUT the tool_use block.
    assert_eq!(ctx.messages.len(), 1);
    assert_eq!(ctx.messages[0], Message::assistant("partial".into(), &[]));
}

#[test]
fn invariant_1_abort_mid_execution_still_backfills_every_result() {
    /// First execute succeeds and flips the shared signal, so the loop
    /// breaks before reaching t2 — t2 must still get a result.
    struct FlipOnSecond<'a>(&'a AtomicBool, AtomicBool);
    impl Tool for FlipOnSecond<'_> {
        fn name(&self) -> &'static str {
            "echo_tool"
        }
        fn description(&self) -> String {
            "echoes".into()
        }
        fn parameters(&self) -> serde_json::Value {
            json!({"type": "object"})
        }
        fn execute(
            &self,
            _args: &serde_json::Value,
            _signal: &AtomicBool,
        ) -> Result<String, tools::ToolError> {
            let _ = self.1.load(Ordering::Relaxed);
            self.0.store(true, Ordering::Relaxed);
            Ok("echo!".into())
        }
    }

    let backend = Shared(RefCell::new(Scripted::new(vec![vec![
        call("t1", "echo_tool"),
        call("t2", "echo_tool"),
        StreamEvent::Done {
            stop_reason: StopReason::ToolUse,
        },
    ]])));
    // Leak so the tool satisfies Box<dyn Tool>'s 'static bound.
    let signal: &'static AtomicBool = Box::leak(Box::new(AtomicBool::new(false)));
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(FlipOnSecond(signal, AtomicBool::new(false)))];
    let mut ctx = Context::default();
    let events = run_agent(&backend, &model(), &mut ctx, &tools, &signal, None);

    let results: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::ToolResult { id, result, .. } => Some((id.as_str(), result.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(results, [("t1", "echo!"), ("t2", "error: aborted")]);
}

/// A user message of exactly 1000 chars, tagged by index so a retained
/// window is identifiable. 1000 divides the char budgets evenly, which
/// makes every cut in these tests exact rather than approximate.
fn filler(tag: usize) -> Message {
    Message::user_text(format!("{tag:04}{}", "x".repeat(996)))
}

fn bulk(n: usize) -> Vec<Message> {
    (0..n).map(filler).collect()
}

#[test]
fn compaction_cut_spends_the_recent_char_budget() {
    let msgs = bulk(200);
    // 1000 chars per message: a 30_000-char window is the newest 30.
    assert_eq!(select_compaction_cut(&msgs, 30_000), 170);
    // 2_500 buys two whole messages; the third would overflow.
    assert_eq!(select_compaction_cut(&msgs, 2_500), 198);
    // Whole context fits the window → nothing eligible to compact.
    assert_eq!(select_compaction_cut(&msgs, 1_000_000), 0);
    // Newest messages larger than the budget still keep the floor.
    assert_eq!(
        select_compaction_cut(&msgs, 0),
        msgs.len() - KEEP_RECENT_MIN
    );
    // Degenerate inputs stay no-ops instead of panicking.
    assert_eq!(select_compaction_cut(&[], 30_000), 0);
    assert_eq!(select_compaction_cut(&msgs[..1], 0), 0);
}

#[test]
fn compaction_cut_skips_orphan_tool_results() {
    // Invariant 1: a kept window may not start on tool_results whose
    // tool_use blocks are about to be summarized away.
    let calls = [ToolCallSpec {
        id: "t1".into(),
        name: "echo_tool".into(),
        args: json!({}),
    }];
    let msgs = vec![
        filler(0),
        Message::assistant("thinking".into(), &calls),
        Message::tool_results(&[("t1".into(), "done".into())]),
        filler(3),
        filler(4),
    ];
    // Budget buys the two fillers plus exactly the tool_results message,
    // so the raw cut lands on index 2 and must advance to 3.
    let budget = 2_000 + message_chars(&msgs[2]);
    assert_eq!(select_compaction_cut(&msgs, budget), 3);
}

#[test]
fn compaction_skipped_below_char_threshold() {
    let backend = Shared(RefCell::new(Scripted::new(vec![vec![
        StreamEvent::TextDelta("hi".into()),
        StreamEvent::Done {
            stop_reason: StopReason::EndTurn,
        },
    ]])));
    // 100 messages = 100_000 chars: far past the old 50-message trigger,
    // still under the char budget → no summary call at all.
    // Caller-provided prompt must win — see run_agent_streaming contract:
    // only fills system_prompt when caller left it None.
    let caller_prompt = "test caller prompt";
    let mut ctx = Context {
        system_prompt: Some(caller_prompt.into()),
        messages: bulk(100),
    };
    let before = ctx.messages.clone();
    let _ = run_agent(&backend, &model(), &mut ctx, &[], &sig(), None);

    let seen = backend.0.borrow();
    assert_eq!(seen.seen_contexts.len(), 1, "summary stream was issued");
    assert_eq!(
        seen.seen_contexts[0].system_prompt.as_deref(),
        Some(caller_prompt),
        "caller-provided prompt must not be overwritten by run_agent_streaming"
    );
    assert_eq!(ctx.messages.len(), before.len() + 1);
    assert!(ctx.messages.iter().take(before.len()).eq(before.iter()));
}

#[test]
fn invariant_4_compaction_failure_keeps_context() {
    // First call = oversized context triggers compaction which errors;
    // second call = the normal turn must see ALL original messages intact.
    let turns = vec![
        vec![StreamEvent::Error("summary backend down".into())],
        vec![
            StreamEvent::TextDelta("ok".into()),
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            },
        ],
    ];
    let backend = Shared(RefCell::new(Scripted::new(turns)));

    // 200_000 chars: above the char budget.
    let mut ctx = Context {
        system_prompt: None,
        messages: bulk(200),
    };
    let before = ctx.clone();
    let _ = run_agent(&backend, &model(), &mut ctx, &[], &sig(), None);

    // Compaction failed → no summary message injected; originals intact.
    assert_eq!(ctx.messages.len(), before.messages.len() + 1); // + final assistant msg
    assert!(ctx.messages.iter().take(200).eq(before.messages.iter()));
}

#[test]
fn run_agent_injects_default_system_prompt_when_caller_leaves_none() {
    // run_agent_streaming contract: when caller doesn't set
    // `Context.system_prompt`, orbit fills it with the main-agent
    // profile lifted from `crates/prompts/prompts/agents/main.md`
    // (oh-my-pi `prompts/agents/task.md`). The whole file —
    // frontmatter included — is the prompt; no concatenation.
    let backend = Shared(RefCell::new(Scripted::new(vec![vec![
        StreamEvent::TextDelta("hi".into()),
        StreamEvent::Done {
            stop_reason: StopReason::EndTurn,
        },
    ]])));
    let mut ctx = Context {
        system_prompt: None,
        messages: vec![Message::user_text("hi")],
    };
    let tools: Vec<Box<dyn tools::Tool>> = vec![Box::new(tools::read::ReadFile)];
    let _ = run_agent_streaming(
        &backend,
        &model(),
        &mut ctx,
        &tools,
        &sig(),
        LoopConfig::default(),
        &mut |_| {},
    );

    let seen = backend.0.borrow();
    assert_eq!(seen.seen_contexts.len(), 1);
    let prompt = seen.seen_contexts[0]
        .system_prompt
        .as_deref()
        .expect("system_prompt should be filled when caller left None");
    // omp-style profile: task.md has no YAML frontmatter (only
    // scout/librarian/reviewer/etc. do). The signal that this is
    // a real role profile is the `<directives>` block, which
    // task.md ships.
    assert!(prompt.starts_with("Worker agent:"));
    assert!(prompt.contains("<directives>"));
}

#[test]
fn compaction_broken_summary_leaves_context_identical() {
    // Error, abort, and empty-summary paths are each a pure no-op.
    let failures = vec![
        vec![StreamEvent::Error("backend down".into())],
        vec![
            StreamEvent::TextDelta("partial".into()),
            StreamEvent::Done {
                stop_reason: StopReason::Aborted,
            },
        ],
        vec![StreamEvent::Done {
            stop_reason: StopReason::EndTurn,
        }],
    ];
    for turn in failures {
        let backend = Shared(RefCell::new(Scripted::new(vec![turn])));
        let mut ctx = Context {
            system_prompt: Some("sys".into()),
            messages: bulk(200),
        };
        let before = ctx.clone();
        orbit::compact_context(&backend, &model(), &mut ctx, &sig(), None);
        assert_eq!(ctx, before);
    }
}

#[test]
fn compaction_keeps_newest_window_on_success() {
    let turns = vec![
        vec![
            StreamEvent::TextDelta("the gist".into()),
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            },
        ],
        vec![StreamEvent::Done {
            stop_reason: StopReason::EndTurn,
        }],
    ];
    let backend = Shared(RefCell::new(Scripted::new(turns)));

    let mut ctx = Context {
        system_prompt: None,
        messages: bulk(200),
    };
    let before = ctx.messages.clone();
    let _ = run_agent(&backend, &model(), &mut ctx, &[], &sig(), None);

    // [summary] + newest window + final assistant msg.
    let kept = KEEP_RECENT_CHARS / 1000;
    assert_eq!(ctx.messages.len(), 1 + kept + 1);
    assert_eq!(
        ctx.messages[0],
        Message::user_text("[context summary]\nthe gist")
    );
    assert!(
        ctx.messages[1..=kept]
            .iter()
            .eq(before[200 - kept..].iter())
    );

    // The summary request carried the old prefix and none of the window.
    let seen = backend.0.borrow();
    let sent = match &seen.seen_contexts[0].messages[0].content {
        adaptor::Content::Text(s) => s.as_str(),
        other => panic!("summary request should be plain text: {other:?}"),
    };
    assert!(sent.contains("0169"), "oldest prefix must be summarized");
    assert!(!sent.contains("0170"), "kept window must not be summarized");
}

#[test]
fn compaction_skipped_when_kept_window_alone_exceeds_budget() {
    // Two newest messages of 200_000 chars each: KEEP_RECENT_MIN pins them
    // in place, so no prefix summary can bring the context under budget.
    // Compacting anyway would summarize the previous summary every turn.
    let backend = Shared(RefCell::new(Scripted::new(vec![vec![
        StreamEvent::TextDelta("ok".into()),
        StreamEvent::Done {
            stop_reason: StopReason::EndTurn,
        },
    ]])));
    let mut msgs = bulk(5);
    msgs.push(Message::user_text("A".repeat(200_000)));
    msgs.push(Message::user_text("B".repeat(200_000)));
    // The cut is nonzero — the guard, not `keep_at == 0`, must stop this.
    assert_eq!(
        select_compaction_cut(&msgs, KEEP_RECENT_CHARS),
        msgs.len() - KEEP_RECENT_MIN
    );

    let mut ctx = Context {
        system_prompt: None,
        messages: msgs,
    };
    let before = ctx.messages.clone();
    let _ = run_agent(&backend, &model(), &mut ctx, &[], &sig(), None);

    // Exactly one backend call: the regular turn, never a summary request.
    let seen = backend.0.borrow();
    assert_eq!(seen.seen_contexts.len(), 1, "summary stream was issued");
    assert_eq!(seen.seen_contexts[0].messages, before);
    // Originals untouched; only the final assistant reply was appended.
    assert_eq!(ctx.messages.len(), before.len() + 1);
    assert!(ctx.messages.iter().take(before.len()).eq(before.iter()));
}

#[test]
fn compaction_skipped_when_system_prompt_alone_exceeds_budget() {
    let backend = Shared(RefCell::new(Scripted::new(vec![vec![
        StreamEvent::TextDelta("summary should not be requested".into()),
        StreamEvent::Done {
            stop_reason: StopReason::EndTurn,
        },
    ]])));
    let mut ctx = Context {
        system_prompt: Some("s".repeat(110_000)),
        messages: vec![Message::user_text("m".repeat(25_000))],
    };
    let before = ctx.messages.clone();

    orbit::compact_context(&backend, &model(), &mut ctx, &sig(), None);

    assert!(backend.0.borrow().seen_contexts.is_empty());
    assert_eq!(ctx.messages, before);
}

#[test]
fn context_log_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("sub/ctx.jsonl");
    let backend = Shared(RefCell::new(Scripted::new(vec![
        vec![
            call("t1", "echo_tool"),
            StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
            },
        ],
        vec![StreamEvent::Done {
            stop_reason: StopReason::EndTurn,
        }],
    ])));
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];
    let mut ctx = Context::default();
    let log = ContextLog::new(&log_path);
    let _ = run_agent(&backend, &model(), &mut ctx, &tools, &sig(), Some(&log));

    let replayed = ContextLog::load(&log_path).unwrap();
    assert_eq!(replayed.len(), ctx.messages.len());
    assert_eq!(replayed, ctx.messages);
}

#[test]
fn turn_cap_stops_runaway_tool_loops() {
    // Every turn returns another tool_call: the old loop only ended when
    // the model felt like it, so a runaway backend burned provider budget.
    let turns = (0..10)
        .map(|i| {
            vec![
                call(&format!("t{i}"), "echo_tool"),
                StreamEvent::Done {
                    stop_reason: StopReason::ToolUse,
                },
            ]
        })
        .collect();
    let backend = Shared(RefCell::new(Scripted::new(turns)));
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];
    let mut ctx = Context::default();
    let mut events = Vec::new();
    run_agent_streaming(
        &backend,
        &model(),
        &mut ctx,
        &tools,
        &sig(),
        LoopConfig {
            max_turns: 3,
            ..LoopConfig::default()
        },
        &mut |e| events.push(e),
    );

    // Exactly 3 LLM round-trips, then a MaxTurns stop — not a 4th call.
    assert_eq!(backend.0.borrow().calls_made, 3);
    assert_eq!(
        events.last(),
        Some(&AgentEvent::TurnEnd {
            stop_reason: TurnStop::MaxTurns
        })
    );
}

#[test]
fn tool_start_precedes_each_execution() {
    let backend = Shared(RefCell::new(Scripted::new(vec![vec![
        call("t1", "echo_tool"),
        call("t2", "echo_tool"),
        StreamEvent::Done {
            stop_reason: StopReason::ToolUse,
        },
    ]])));
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];
    let mut ctx = Context::default();
    let events = run_agent(&backend, &model(), &mut ctx, &tools, &sig(), None);

    let starts: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::ToolStart { id, name } => Some((id.as_str(), name.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(starts, [("t1", "echo_tool"), ("t2", "echo_tool")]);
    // Each dispatch start lands before its matching result.
    let first_start = events
        .iter()
        .position(|e| matches!(e, AgentEvent::ToolStart { id, .. } if id == "t1"))
        .unwrap();
    let first_result = events
        .iter()
        .position(|e| matches!(e, AgentEvent::ToolResult { id, .. } if id == "t1"))
        .unwrap();
    assert!(first_start < first_result);
}

#[test]
fn compaction_summary_is_logged_for_replay() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("ctx.jsonl");
    let turns = vec![
        vec![
            StreamEvent::TextDelta("the gist".into()),
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            },
        ],
        vec![StreamEvent::Done {
            stop_reason: StopReason::EndTurn,
        }],
    ];
    let backend = Shared(RefCell::new(Scripted::new(turns)));
    let initial = bulk(200);
    let mut ctx = Context {
        system_prompt: None,
        messages: initial.clone(),
    };
    let log = ContextLog::new(&log_path);
    // Messages predating the run must be logged by the caller (the
    // runner does this for the user prompt); the loop only records
    // what it produces itself.
    for m in &initial {
        log.append_message(m).unwrap();
    }
    let _ = run_agent(&backend, &model(), &mut ctx, &[], &sig(), Some(&log));

    // Replay shows the full pre-compaction conversation, then the
    // summary marker, then the final reply. The kept window needs no
    // second copy in the log: those messages ARE the originals at
    // positions 170..200, logged before the run started.
    let replayed = ContextLog::load(&log_path).unwrap();
    assert!(replayed.starts_with(initial.as_slice()));
    assert_eq!(
        replayed[200],
        Message::user_text("[context summary]\nthe gist"),
        "summary marker must sit between the summarized prefix and what follows"
    );
    assert_eq!(replayed.len(), 200 + 1 + 1);
    // In-memory context = [summary] + kept originals + final reply.
    let kept = KEEP_RECENT_CHARS / 1000;
    assert_eq!(ctx.messages.len(), 1 + kept + 1);
    assert_eq!(ctx.messages[0], replayed[200]);
    assert_eq!(ctx.messages[1..=kept], initial[200 - kept..]);
    assert_eq!(ctx.messages[kept + 1], replayed[201]);
}

#[test]
fn steering_drains_into_the_next_round() {
    let backend = Shared(RefCell::new(Scripted::new(vec![
        vec![
            call("t1", "echo_tool"),
            StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
            },
        ],
        vec![
            StreamEvent::TextDelta("done".into()),
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            },
        ],
    ])));
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];
    // Steering queue: one message delivered from the second drain onwards —
    // the user interjected while the tool batch was running. The first
    // drain (loop entry) must come back empty.
    let queue = RefCell::new(vec![Message::user_text("keep it short")]);
    let round = std::cell::Cell::new(0usize);
    let dir = tempfile::tempdir().unwrap();
    let log = ContextLog::new(dir.path().join("ctx.jsonl"));
    let mut ctx = Context {
        system_prompt: Some("sys".into()),
        messages: vec![Message::user_text("start")],
    };
    let get = || {
        round.set(round.get() + 1);
        if round.get() >= 2 {
            std::mem::take(&mut *queue.borrow_mut())
        } else {
            vec![]
        }
    };
    run_agent_streaming(
        &backend,
        &model(),
        &mut ctx,
        &tools,
        &sig(),
        LoopConfig {
            context_log: Some(&log),
            get_steering: Some(&get),
            ..LoopConfig::default()
        },
        &mut |_| {},
    );

    let seen = backend.0.borrow();
    // Round 1 saw only the initial prompt; round 2 saw the steering message
    // appended after the round-1 exchange (assistant reply + tool results).
    assert_eq!(seen.seen_contexts[0].messages.len(), 1);
    assert_eq!(seen.seen_contexts[1].messages.len(), 4);
    assert_eq!(
        seen.seen_contexts[1].messages[3],
        Message::user_text("keep it short")
    );
    // The drained message is part of the final context and the evidence log.
    assert!(ctx.messages.contains(&Message::user_text("keep it short")));
    let replayed = ContextLog::load(dir.path().join("ctx.jsonl")).unwrap();
    assert!(replayed.contains(&Message::user_text("keep it short")));
}

#[test]
fn follow_up_extends_the_run_past_model_endturn() {
    let backend = Shared(RefCell::new(Scripted::new(vec![
        // Model says "first" and stops; an async job result lands right after.
        vec![
            StreamEvent::TextDelta("first".into()),
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            },
        ],
        vec![
            StreamEvent::TextDelta("second".into()),
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            },
        ],
    ])));
    let queue = RefCell::new(vec![Message::user_text("job result: ok")]);
    let mut ctx = Context {
        system_prompt: Some("sys".into()),
        messages: vec![Message::user_text("start")],
    };
    let mut events = Vec::new();
    let get = || std::mem::take(&mut *queue.borrow_mut());
    run_agent_streaming(
        &backend,
        &model(),
        &mut ctx,
        &[],
        &sig(),
        LoopConfig {
            get_follow_up: Some(&get),
            ..LoopConfig::default()
        },
        &mut |e| events.push(e),
    );

    let seen = backend.0.borrow();
    assert_eq!(seen.calls_made, 2, "follow-up must trigger a second round");
    assert_eq!(
        seen.seen_contexts[1].messages.last(),
        Some(&Message::user_text("job result: ok"))
    );
    // Exactly one TurnEnd, only after the queue drained.
    let ends: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, AgentEvent::TurnEnd { .. }))
        .collect();
    assert_eq!(ends.len(), 1);
    assert_eq!(
        events.last(),
        Some(&AgentEvent::TurnEnd {
            stop_reason: TurnStop::EndTurn
        })
    );
}
