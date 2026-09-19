//! Integration tests for subagent run lifecycle: run ids, `interrupt`, and
//! the `subagent_control` interrupt action.
//!
//! The fork worker only polls the abort signal between turns, so the backend
//! below busy-waits inside `stream_cb` until a gate is released: the run is
//! live for exactly as long as the test says it is, and the runner re-checks
//! the signal the moment the turn ends. No sleep-based timing assumptions.

use omenic_harness_tools::Tool;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use adaptor::{Context, Model, StreamEvent, ToolDef};
use orbit::LlmBackend;

use omenic_harness_core::AbortSignal;
use omenic_harness_subagent::tool_subagent_control::SubagentControlTool;
use omenic_harness_subagent::{
    ForkProvider, SubagentResult, SubagentRuntimeService, SubagentStartRequest,
};

fn model() -> Model {
    Model {
        api_key: "test".into(),
        model: "test-model".into(),
        base_url: None,
        max_tokens: None,
    }
}

/// A fork provider whose worker is parked inside the model call until `gate`
/// is set. That parking is what makes interrupt deterministic rather than a
/// race against the worker's startup.
fn fork_service(gate: &Arc<AtomicBool>) -> Arc<SubagentRuntimeService> {
    let service = Arc::new(SubagentRuntimeService::default());
    service.register(
        "fork",
        Arc::new(ForkProvider::new(
            Arc::new(BlockingBackend { gate: gate.clone() }),
            model(),
            // The backend never makes a tool call, so the child tool set is
            // empty and the run still settles on its own once released.
            Arc::new(Vec::new()),
            1,
        )),
    );
    service
}

fn request() -> SubagentStartRequest {
    SubagentStartRequest {
        prompt: "Work forever".into(),
        signal: Arc::new(AtomicBool::new(false)),
        inherits_parent_context: false,
    }
}

struct BlockingBackend {
    gate: Arc<AtomicBool>,
}

impl LlmBackend for BlockingBackend {
    fn stream_cb(
        &self,
        _model: &Model,
        _context: &Context,
        _tools: &[ToolDef],
        _signal: &AtomicBool,
        emit: &mut dyn FnMut(&StreamEvent),
    ) {
        while !self.gate.load(std::sync::atomic::Ordering::Relaxed) {
            std::thread::yield_now();
        }
        emit(&StreamEvent::Done {
            stop_reason: adaptor::StopReason::EndTurn,
        });
    }
}

/// 1. `start_run` hands out sequential ids and retires runs once they settle.
#[test]
fn start_run_ids_are_sequential() {
    let gate = Arc::new(AtomicBool::new(false));
    let service = fork_service(&gate);

    let (id1, run1) = service
        .start_run("fork", request())
        .expect("fork provider is registered");
    let (id2, run2) = service
        .start_run("fork", request())
        .expect("fork provider is registered");
    assert_eq!(id1, "sub-1");
    assert_eq!(id2, "sub-2");

    // Release both workers and drain them so the test leaves no parked
    // thread behind.
    gate.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = run1.result();
    let _ = run2.result();
    service.finish_run(&id1);
    service.finish_run(&id2);
    assert!(service.active_runs().is_empty(), "settled runs must retire");
}

/// 2. `interrupt` disposes a live run, which then settles on `Aborted`.
#[test]
fn interrupt_aborts_live_run() {
    let gate = Arc::new(AtomicBool::new(false));
    let service = fork_service(&gate);

    let (id, run) = service
        .start_run("fork", request())
        .expect("fork provider is registered");
    assert!(
        service.active_runs().contains(&id),
        "run must be live in flight"
    );

    assert!(service.interrupt(&id), "a live run must interrupt");
    // Release the worker: the runner re-checks the signal after the turn and
    // reports the abort even though the turn ended normally.
    gate.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(matches!(run.result(), SubagentResult::Aborted));
    assert!(
        !service.interrupt(&id),
        "an interrupted run is no longer live"
    );
}

/// 3. An unknown run id interrupts nothing.
#[test]
fn interrupt_unknown_run_is_false() {
    let service = SubagentRuntimeService::default();
    assert!(!service.interrupt("nope"), "unknown id must not interrupt");
}

/// 4. `subagent_control` interrupts a live run, reports the outcome in its
///    payload, and a missing `run_id` is a tool error.
#[test]
fn control_tool_interrupts_live_run() {
    let gate = Arc::new(AtomicBool::new(false));
    let service = fork_service(&gate);
    let control = SubagentControlTool::new("subagent_control".into(), service.clone());

    let (id, run) = service
        .start_run("fork", request())
        .expect("fork provider is registered");

    let out = control
        .execute(
            &serde_json::json!({"action": "interrupt", "run_id": &id}),
            &AbortSignal::new(),
        )
        .expect("interrupt is a supported action");
    assert!(
        !out.is_error,
        "an interrupt outcome is reported, not an error"
    );
    let payload: serde_json::Value = serde_json::from_str(&out.output).expect("json payload");
    assert_eq!(payload["interrupted"], true);
    assert_eq!(payload["run_id"], id);

    gate.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(matches!(run.result(), SubagentResult::Aborted));

    // The run has settled: `finish_run` retires it and a later interrupt on
    // the same id is a no-op.
    service.finish_run(&id);
    assert!(!service.interrupt(&id));

    // An unknown run is a payload failure, not a tool execution error.
    let out = control
        .execute(
            &serde_json::json!({"action": "interrupt", "run_id": "ghost"}),
            &AbortSignal::new(),
        )
        .unwrap();
    assert!(!out.is_error);
    let payload: serde_json::Value = serde_json::from_str(&out.output).expect("json payload");
    assert_eq!(payload["interrupted"], false);
    assert_eq!(payload["error"], "no such run");

    // `interrupt` without `run_id` is a malformed call, so it is an error.
    let err = control
        .execute(
            &serde_json::json!({"action": "interrupt"}),
            &AbortSignal::new(),
        )
        .expect_err("run_id is required for interrupt");
    assert!(
        err.to_string().contains("missing string argument: run_id"),
        "error must name the missing argument, got: {err}"
    );
}
