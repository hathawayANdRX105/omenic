use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use agent_loop::orbit::LlmBackend;
use llm::{Context, Model, StreamEvent, ToolDef};
use tools::Tool;

use plugin::{DshPlugin, Fiber};
use subagent::{
    ForkProvider, SubagentProvider, SubagentResult, SubagentRuntime, SubagentRuntimeService,
};

fn model() -> Model {
    Model {
        api_key: "test".into(),
        model: "test-model".into(),
        base_url: None,
        max_tokens: None,
    }
}

fn read_only_tools() -> Arc<Vec<Box<dyn Tool>>> {
    Arc::new(vec![
        Box::new(tools::read::ReadFile),
        Box::new(tools::grep::Grep),
        Box::new(tools::glob::Glob),
    ])
}

/// Scripted backend that replays canned turns per `stream_cb` invocation.
struct ScriptedBackend {
    turns: Vec<Vec<StreamEvent>>,
    call: std::sync::Mutex<usize>,
}

impl ScriptedBackend {
    fn new(turns: Vec<Vec<StreamEvent>>) -> Self {
        Self {
            turns,
            call: std::sync::Mutex::new(0),
        }
    }
}

impl LlmBackend for ScriptedBackend {
    fn stream_cb(
        &self,
        _model: &Model,
        _context: &Context,
        _tools: &[ToolDef],
        _signal: &AtomicBool,
        emit: &mut dyn FnMut(&StreamEvent),
    ) {
        let mut call = self.call.lock().unwrap();
        let turn = self.turns.get(*call).cloned().unwrap_or_else(|| {
            vec![StreamEvent::Done {
                stop_reason: llm::StopReason::EndTurn,
            }]
        });
        *call += 1;
        for ev in turn {
            emit(&ev);
        }
    }
}

/// Backend that blocks inside `stream_cb` until the test explicitly
/// releases it. This guarantees the worker is still running when
/// `dispose` is called.
struct BlockingBackend {
    inner: ScriptedBackend,
    gate: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl BlockingBackend {
    fn new(
        turns: Vec<Vec<StreamEvent>>,
        gate: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        Self {
            inner: ScriptedBackend::new(turns),
            gate,
        }
    }
}

impl LlmBackend for BlockingBackend {
    fn stream_cb(
        &self,
        model: &Model,
        context: &Context,
        tools: &[ToolDef],
        signal: &AtomicBool,
        emit: &mut dyn FnMut(&StreamEvent),
    ) {
        // Busy-wait until the test releases us. This keeps the worker
        // inside `run_subagent` so `dispose` can flip the signal.
        while !self.gate.load(std::sync::atomic::Ordering::Relaxed) {
            std::thread::yield_now();
        }
        self.inner.stream_cb(model, context, tools, signal, emit);
    }
}

/// 1. A fork run completes and yields `Completed` with non-empty output.
#[test]
fn fork_provider_returns_run() {
    let backend = Arc::new(ScriptedBackend::new(vec![vec![
        StreamEvent::TextDelta("hello from subagent".into()),
        StreamEvent::Done {
            stop_reason: llm::StopReason::EndTurn,
        },
    ]]));
    let provider = ForkProvider::new(backend, model(), read_only_tools(), 1);
    let signal = Arc::new(AtomicBool::new(false));
    let request = subagent::SubagentStartRequest {
        prompt: "Say hello".into(),
        signal,
        inherits_parent_context: true,
        inbox: None,
    };
    let run = provider.start(request);
    let result = run.result();
    match result {
        SubagentResult::Completed { output } => assert!(!output.is_empty()),
        _ => panic!("expected Completed, got {result:?}"),
    }
}

/// 2. `SubagentRuntime` registers `SubagentRuntimeService` in a `Fiber`,
///    and the service resolves under the key `"harness.subagents"`.
#[test]
fn runtime_registers_provider() {
    let mut fiber = Fiber::default();
    let runtime = SubagentRuntime::default();
    let mut ctx = fiber.context();
    runtime.register(&mut ctx);
    drop(ctx);
    let service: Option<Arc<SubagentRuntimeService>> = fiber.resolve("harness.subagents");
    assert!(
        service.is_some(),
        "SubagentRuntimeService must be registered"
    );
    let names = service.unwrap().providers();
    assert!(names.is_empty(), "no providers registered yet");
}

/// 3. `dispose` flips the abort signal and the worker thread settles on
///    `SubagentResult::Aborted`.
#[test]
fn dispose_aborts_running_subagent() {
    let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let backend = Arc::new(BlockingBackend::new(
        vec![vec![StreamEvent::Done {
            stop_reason: llm::StopReason::MaxTokens,
        }]],
        gate.clone(),
    ));
    let provider = ForkProvider::new(backend, model(), read_only_tools(), 1);
    let signal = Arc::new(AtomicBool::new(false));
    let request = subagent::SubagentStartRequest {
        prompt: "Work forever".into(),
        signal: signal.clone(),
        inherits_parent_context: true,
        inbox: None,
    };
    let run = provider.start(request);
    // Worker is now blocked inside stream_cb. dispose flips the signal
    // while the worker is still inside run_subagent.
    run.dispose();
    // Release the worker so it can observe the signal and return.
    gate.store(true, std::sync::atomic::Ordering::Relaxed);
    let result = run.result();
    assert!(matches!(result, SubagentResult::Aborted));
}
