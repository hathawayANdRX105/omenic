//! G6 — the assembled container is consumed, not just registered.
//!
//! The daemon worker used to hardcode `tools::builtin_tools()` and
//! `orbit::LoopConfig::default()`. These tests drive the replacement path:
//! assemble the container over a config document, resolve the services the
//! plugins provided, build an [`rpc::worker::OrbitSetup`] from them, and run
//! a worker (or the loop directly) against a scripted backend.
//!
//! What is under test:
//! 1. `cwd` from the config document reaches the loop as
//!    `instruction_cwd` — an `AGENTS.md` in the workspace is injected into
//!    the system prompt the backend receives.
//! 2. `max_turns` from the document flows document -> `harness.loop` ->
//!    resolved -> `OrbitConfig` -> `LoopConfig::max_turns`, and the loop
//!    stops on the cap.
//! 3. `harness.compaction` is resolved and drives the maintenance hook — a
//!    >120k-char session compacts with the tool-pairing invariant intact.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use llm::{Content, Context, Message, Model, StopReason, StreamEvent};
use orbit::{LlmBackend, LoopConfig, run_agent_streaming};
use protocol::events::ToolCallSpec;
use protocol::events::{AgentEvent, TurnStop};
use rpc::worker::{OrbitConfig, OrbitSetup, WorkerEvent};
use serde_json::json;
use tempfile::tempdir;

/// Distinctive line written into the workspace `AGENTS.md`.
const MARKER: &str = "<!-- g6-container-marker -->";

fn model() -> Model {
    Model {
        api_key: "k".into(),
        model: "test".into(),
        base_url: None,
        max_tokens: None,
    }
}

/// Scripted backend: replays canned event lists per call (the loop.rs /
/// compaction_e2e pattern), recording every context handed to it. `Mutex`
/// instead of `RefCell` because the worker runs the loop on its own thread
/// and the trait object must be `Send + Sync`.
struct Scripted {
    turns: Vec<Vec<StreamEvent>>,
    calls: usize,
    seen: Vec<Context>,
}

struct Shared(Mutex<Scripted>);

impl Shared {
    fn new(turns: Vec<Vec<StreamEvent>>) -> Self {
        Shared(Mutex::new(Scripted {
            turns,
            calls: 0,
            seen: Vec::new(),
        }))
    }
}

impl LlmBackend for Shared {
    fn stream_cb(
        &self,
        _model: &Model,
        context: &Context,
        _tools: &[llm::ToolDef],
        _signal: &AtomicBool,
        emit: &mut dyn FnMut(&StreamEvent),
    ) {
        let mut s = self.0.lock().expect("scripted backend lock");
        s.seen.push(context.clone());
        let turn = match s.turns.get(s.calls) {
            Some(t) => t.clone(),
            // Turns exhausted: end cleanly so the loop always terminates.
            None => vec![StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            }],
        };
        s.calls += 1;
        drop(s);
        for ev in &turn {
            emit(ev);
        }
    }
}

/// Assemble the container over a config document and resolve the three
/// services the worker consumes — the same resolutions
/// `Daemon::orbit_setup` performs in production. Panics when a service is
/// missing: G6's contract is that the core plugins provide them.
fn container(
    doc: serde_json::Value,
) -> (
    Arc<tools_harness::ToolCatalog>,
    Arc<compaction::CharBudgetPolicy>,
    usize,
) {
    let (fiber, _registry) = composition::assemble(doc, vec![]).expect("assemble");
    let catalog = fiber
        .resolve::<tools_harness::ToolCatalog>("harness.tools")
        .expect("harness.tools provided by the tools step");
    let compaction = fiber
        .resolve::<compaction::CharBudgetPolicy>("harness.compaction")
        .expect("harness.compaction provided by CompactionPlugin");
    let max_turns = fiber
        .resolve::<agent_loop::LoopEngine>("harness.loop")
        .expect("harness.loop provided by the runtime step")
        .max_turns;
    (catalog, compaction, max_turns)
}

/// Build the setup the worker takes, resolving services from a document.
fn setup(doc: serde_json::Value, backend: Arc<Shared>) -> OrbitSetup {
    let cwd = doc
        .get("cwd")
        .and_then(serde_json::Value::as_str)
        .map(|c| Arc::<Path>::from(Path::new(c)));
    let (catalog, compaction, max_turns) = container(doc);
    OrbitSetup {
        model: model(),
        backend,
        config: OrbitConfig {
            cwd,
            max_turns,
            compaction,
            catalog,
            // No MCP servers under test here: empty list = pre-MCP behavior.
            mcp_tools: std::sync::Arc::new(Vec::new()),
            // Likewise no job/terminal tools: empty list = the engine's tool
            // list is exactly what it was before that family existed.
            session_tools: std::sync::Arc::new(Vec::new()),
            plan_policy_section: None,
        },
    }
}

/// One turn that streams `text` and ends the turn.
fn answer_turn(text: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::TextDelta(text.into()),
        StreamEvent::Done {
            stop_reason: StopReason::EndTurn,
        },
    ]
}

/// Drain the worker's event stream until `AgentEnd`, collecting events.
fn drain_until_end(rx: &std::sync::mpsc::Receiver<WorkerEvent>) -> Vec<WorkerEvent> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.recv_timeout(Duration::from_secs(10)) {
        out.push(ev.clone());
        if matches!(ev, WorkerEvent::AgentEnd { .. }) {
            break;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 1. AGENTS.md injection reaches the backend in the production path
// ---------------------------------------------------------------------------

/// A daemon started under a workspace with an `AGENTS.md` hands the loop the
/// session cwd; the rendered instruction fragment must reach the system
/// prompt the backend actually receives — the first time this is true in the
/// daemon worker path (G6).
#[test]
fn agents_md_from_container_cwd_reaches_the_backend() {
    let root = tempdir().expect("temp workspace");
    let agents_md = root.path().join("AGENTS.md");
    std::fs::write(&agents_md, format!("{MARKER}\nbe excellent to each other")).unwrap();

    let backend = Arc::new(Shared::new(vec![answer_turn("ok")]));
    let mut worker = rpc::worker::Worker::new(
        "unused-in-orbit-mode",
        Some(setup(
            json!({ "cwd": root.path().to_string_lossy() }),
            Arc::clone(&backend),
        )),
    )
    .expect("orbit worker");

    let rx = worker.subscribe("test");
    worker.prompt("hello").expect("prompt accepted");
    let events = drain_until_end(&rx);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, WorkerEvent::AgentEnd { .. })),
        "run must terminate"
    );

    let s = backend.0.lock().unwrap();
    assert_eq!(s.calls, 1, "exactly one LLM round-trip");
    let prompt = s.seen[0].system_prompt.as_deref().unwrap_or("");
    let heading = format!("Instructions from: {}", agents_md.display());
    assert!(
        prompt.starts_with(heading.as_str()),
        "prompt must lead with the rendered AGENTS.md section, got: {:?}",
        prompt.chars().take(160).collect::<String>()
    );
    assert!(
        prompt.contains(MARKER),
        "injected content must reach the model"
    );
    // The role profile survives the prefix.
    assert!(prompt.contains("Worker agent:"));
}

/// Opt-out holds in the worker path too: no `cwd` in the document, no
/// discovery — the backend sees the bare role profile.
#[test]
fn no_cwd_in_document_keeps_the_bare_profile() {
    let backend = Arc::new(Shared::new(vec![answer_turn("ok")]));
    let mut worker =
        rpc::worker::Worker::new("unused", Some(setup(json!({}), Arc::clone(&backend))))
            .expect("orbit worker");
    let rx = worker.subscribe("test");
    worker.prompt("hi").unwrap();
    drain_until_end(&rx);

    let s = backend.0.lock().unwrap();
    let prompt = s.seen[0].system_prompt.as_deref().unwrap_or("");
    assert!(
        !prompt.contains(MARKER),
        "no cwd handed over, no instruction lookup"
    );
    assert!(prompt.contains("Worker agent:"));
}

// ---------------------------------------------------------------------------
// 2. max_turns from the config document caps the run
// ---------------------------------------------------------------------------

/// One turn that issues a tool call and stops with `ToolUse`: the model never
/// ends the turn on its own, so the loop's only exit is the turn cap.
fn tool_turn() -> Vec<StreamEvent> {
    vec![
        StreamEvent::ToolCall(ToolCallSpec {
            id: "t1".into(),
            name: "no_such_tool".into(),
            args: json!({}),
        }),
        StreamEvent::Done {
            stop_reason: StopReason::ToolUse,
        },
    ]
}

/// The turn cap flows document -> `harness.loop` -> worker -> loop: the run
/// stops after exactly `max_turns` rounds even though the model keeps calling
/// tools. `WorkerEvent` collapses every `TurnEnd` into `AgentEnd` (carrying
/// the stop reason), so the cap is observed as the count of `AgentStart`
/// rounds before the run ends — the exact `MaxTurns` variant is asserted in
/// the loop-level test below.
#[test]
fn container_max_turns_caps_the_worker_run() {
    // More tool-turns than the cap: the model never ends a turn on its own, so
    // the loop's only exit is the turn cap.
    let backend = Arc::new(Shared::new(vec![tool_turn(); 8]));
    let mut worker = rpc::worker::Worker::new(
        "unused",
        Some(setup(json!({ "max_turns": 3 }), Arc::clone(&backend))),
    )
    .expect("orbit worker");
    let rx = worker.subscribe("test");
    worker.prompt("keep going").unwrap();
    let events = drain_until_end(&rx);

    let rounds = events
        .iter()
        .filter(|e| matches!(e, WorkerEvent::AgentStart))
        .count();
    assert_eq!(rounds, 3, "exactly max_turns LLM rounds before the cap");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, WorkerEvent::AgentEnd { .. })),
        "the capped run must still end cleanly"
    );
    // The backend was called exactly max_turns times — the 4th round the
    // model asked for never happened.
    assert_eq!(backend.0.lock().unwrap().calls, 3);
}

/// Loop-level assertion of the same wiring, where the exact stop reason is
/// observable: the `LoopConfig` built from the resolved container services
/// emits `TurnEnd { MaxTurns }` once the cap is hit.
#[test]
fn loop_reports_max_turns_from_container_config() {
    let (_catalog, _compaction, max_turns) = container(json!({ "max_turns": 3 }));
    assert_eq!(
        max_turns, 3,
        "max_turns must round-trip through the container"
    );

    // Enough tool-turns to outlast the cap: the loop must stop on max_turns,
    // not on the model ending a turn.
    let backend = Shared::new(vec![tool_turn(); 8]);
    let mut ctx = Context {
        system_prompt: Some("sys".into()),
        messages: vec![Message::user_text("keep going")],
    };
    let tools = tools::builtin_tools();
    let mut events = Vec::new();
    run_agent_streaming(
        &backend,
        &model(),
        &mut ctx,
        &tools,
        &AtomicBool::new(false),
        LoopConfig {
            max_turns,
            ..LoopConfig::default()
        },
        &mut |ev| events.push(ev),
    );
    let starts = events
        .iter()
        .filter(|e| matches!(e, AgentEvent::TurnStart))
        .count();
    assert_eq!(starts, 3);
    assert!(matches!(
        events.last(),
        Some(AgentEvent::TurnEnd {
            stop_reason: TurnStop::MaxTurns
        })
    ));
}

// ---------------------------------------------------------------------------
// 3. The resolved compaction policy drives the maintenance hook
// ---------------------------------------------------------------------------

/// Total request size in the policy's own accounting — what `compact_with`
/// measures against the budget.
fn request_chars(ctx: &Context) -> usize {
    let system = ctx.system_prompt.as_deref().map_or(0, str::len);
    system + ctx.messages.iter().map(orbit::message_chars).sum::<usize>()
}

/// (tool_use ids, tool_result ids) carried by the whole message list.
fn pairing_ids(
    msgs: &[Message],
) -> (
    std::collections::HashSet<String>,
    std::collections::HashSet<String>,
) {
    let mut uses = std::collections::HashSet::new();
    let mut results = std::collections::HashSet::new();
    for m in msgs {
        let Content::Blocks(bs) = &m.content else {
            continue;
        };
        for b in bs {
            match b {
                llm::Block::ToolUse { id, .. } => {
                    uses.insert(id.clone());
                }
                llm::Block::ToolResult { tool_use_id, .. } => {
                    results.insert(tool_use_id.clone());
                }
                _ => {}
            }
        }
    }
    (uses, results)
}

/// A user message of exactly 1000 chars, tagged so the summarized prefix and
/// the verbatim recent window stay identifiable.
fn filler(tag: usize) -> Message {
    Message::user_text(format!("{tag:04}{}", "x".repeat(996)))
}

/// `harness.compaction` resolved from the container drives the maintenance
/// hook: a session over the policy's budget compacts (the summary marker
/// leads the shipped context, and the request is back under budget) while
/// orbit invariant 1 holds — every tool_use still carries its tool_result.
#[test]
fn container_policy_drives_compaction_without_breaking_pairs() {
    let (_catalog, compaction, max_turns) = container(json!({}));
    let budget = compaction.total_chars();
    assert_eq!(budget, compaction::COMPACT_CHAR_BUDGET);

    // 150k chars of history, then a tool pair that straddles the raw
    // recent-window cut — the exact spot that would orphan the result if the
    // pairing guard didn't advance the cut past it.
    let mut msgs: Vec<Message> = (0..150).map(filler).collect();
    msgs.push(Message::assistant(
        "A".repeat(35_000),
        &[ToolCallSpec {
            id: "t_edge".into(),
            name: "echo".into(),
            args: json!({}),
        }],
    ));
    msgs.push(Message::tool_results(&[(
        "t_edge".into(),
        "edge result".into(),
    )]));
    msgs.extend([150usize, 151].map(filler));
    let mut ctx = Context {
        system_prompt: Some("sys".into()),
        messages: msgs,
    };
    assert!(
        request_chars(&ctx) > budget,
        "fixture must be genuinely oversized"
    );

    // Call #1 is the summary stream (the hook runs before the first turn),
    // call #2 is the turn over the compacted context.
    let backend = Shared::new(vec![answer_turn("the gist"), answer_turn("ok")]);
    let tools = tools::builtin_tools();
    let policy = Arc::clone(&compaction);
    let maintain = |b: &dyn LlmBackend, m: &Model, c: &mut Context, s: &AtomicBool| {
        orbit::compact_context_with(b, m, c, s, None, &policy);
    };
    run_agent_streaming(
        &backend,
        &model(),
        &mut ctx,
        &tools,
        &AtomicBool::new(false),
        LoopConfig {
            max_turns,
            maintain: Some(&maintain),
            ..LoopConfig::default()
        },
        &mut |_| {},
    );

    // Compaction fired: the marker leads, and the request is back in budget.
    assert!(
        matches!(
            ctx.messages.first(),
            Some(m) if matches!(&m.content, Content::Text(s) if s.starts_with(compaction::SUMMARY_PREFIX))
        ),
        "shipped context must lead with the summary marker"
    );
    assert!(
        request_chars(&ctx) < budget,
        "compaction must actually cut the request"
    );
    // Invariant 1: no orphaned tool_use or tool_result survived the cut.
    let (uses, results) = pairing_ids(&ctx.messages);
    assert_eq!(
        uses, results,
        "tool_call/tool_result pairing must stay balanced"
    );
}
