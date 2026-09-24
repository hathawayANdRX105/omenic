//! Runtime engine for the omenic agent harness.
//!
//! Reference: `omenic agent/orbit/src/lib.rs:617` (`run_agent`) and
//! `dsh agent-loop`.
//!
//! orbit's `run_agent_streaming` is not reused verbatim: it speaks the
//! `adaptor` streaming/block types and takes an initial `Context`, none of
//! which the harness `Provider`/`Message` vocabulary carries. This loop is
//! the same transaction (call → tool → backfill → repeat) over the harness
//! traits, keeping orbit's invariants 1 (every tool_call gets a result
//! message) and 5 (the loop bounds round-trips itself).

use protocol::{
    AbortSignal, LlmError, Message, Run, RunError, RunId, RunStatus, Step, StepId, ToolError,
    ToolSpec, new_run,
};
use serde_json::Value;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use tools_harness::ToolExecutor;

/// LLM provider abstraction. Mirrors `orbit::LlmBackend` but
/// uses the harness types.
pub trait Provider {
    fn call(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolSpec],
    ) -> Pin<Box<dyn Future<Output = Result<Message, LlmError>> + Send>>;
}

/// Central loop engine holding runtime config (max turns, compaction,
/// etc.).
#[derive(Debug, Default)]
pub struct LoopEngine {
    pub max_turns: usize,
    /// Model name forwarded to `Provider::call`.
    pub model: String,
    // future fields (maintain, get_steering, etc.) added later
}

/// Park/unpark waker: the harness loop is synchronous, so the only wake
/// it needs is to unpark the driving thread.
struct ThreadWake(std::thread::Thread);

impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

/// Block the current thread driving a provider future to completion.
/// ponytail: no async runtime in this workspace — `Provider` futures are
/// expected to be thin wrappers over blocking transports (like orbit's
/// `ureq`). A real tokio host should call its provider directly, not this loop.
fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        if let Poll::Ready(v) = future.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::park();
    }
}

/// Timestamp-based run id.
/// ponytail: nanosecond clock is collision-free for a single host; switch
/// to a uuid crate if runs ever get generated across parallel processes.
fn next_run_id() -> RunId {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    RunId::new(format!("run-{nanos:x}"))
}

/// Parse a `tool_call` message body: `{"name": ..., "arguments": {...}}`.
/// Returns `None` for anything else (treated as plain assistant text).
fn parse_tool_call(content: &str) -> Option<(String, Value)> {
    let v: Value = serde_json::from_str(content).ok()?;
    let name = v.get("name")?.as_str()?.to_string();
    let args = v
        .get("arguments")
        .cloned()
        .unwrap_or(Value::Object(Default::default()));
    Some((name, args))
}

/// Runs the full agent loop until the model returns a non-`tool_call`
/// message, the abort signal fires, or `engine.max_turns` is exhausted.
///
/// Tool-call wire convention (harness `Message` has no tool-call field):
/// role `tool_call`, content a JSON object `{"name": ..., "arguments":
/// {...}}`; the backfilled result is a `tool` message carrying the output.
///
/// Reference: `omenic agent/orbit/src/lib.rs:617` (`run_agent`) and
/// `dsh agent-loop` core transaction.
/// Constraint: calls the provider, executes tools via `executor`,
/// respects `abort`, and returns a complete `Run` with all steps.
/// Non-goal: no streaming, no compaction policy, no session persistence.
pub fn run_agent_loop(
    engine: &LoopEngine,
    provider: &impl Provider,
    executor: &impl ToolExecutor,
    abort: &AbortSignal,
) -> Result<Run, RunError> {
    let mut run = new_run(next_run_id());
    let specs = executor.specs();
    // ponytail: the loop itself seeds no user turn — a stateful provider
    // owns the initial conversation; the loop only accumulates as it goes.
    let mut messages: Vec<Message> = Vec::new();

    for turn in 0..engine.max_turns {
        if abort.is_aborted() {
            return Err(RunError::Aborted);
        }
        let msg = match block_on(provider.call(&engine.model, &messages, &specs)) {
            Ok(m) => m,
            Err(LlmError::Transport(s)) | Err(LlmError::InvalidResponse(s)) => {
                return Err(RunError::Llm(s));
            }
        };

        let step = |status: RunStatus, summary: String| Step {
            id: StepId::new(format!("step-{turn}")),
            status,
            summary,
        };

        if msg.role != "tool_call" {
            messages.push(msg.clone());
            run.steps.push(step(RunStatus::EndTurn, msg.content));
            return Ok(run);
        }

        let (name, args) = parse_tool_call(&msg.content).ok_or_else(|| {
            RunError::Llm(format!("malformed tool_call content: {}", msg.content))
        })?;
        // Unknown names still go through the executor so the error is
        // backfilled as a value (orbit invariant 1), not run-killing.
        let spec = specs
            .iter()
            .find(|s| s.name == name)
            .cloned()
            .unwrap_or(ToolSpec {
                name: name.clone(),
                description: String::new(),
                params_schema: Value::Null,
            });
        let output = match executor.execute(&spec, &args, abort) {
            Ok(r) => r.output,
            Err(ToolError::Aborted) => return Err(RunError::Aborted),
            Err(e) => format!("tool error: {e}"),
        };
        messages.push(msg);
        messages.push(Message {
            role: "tool".to_string(),
            content: output,
        });
        run.steps
            .push(step(RunStatus::EndTurn, format!("tool:{name}")));
    }

    run.steps.push(Step {
        id: StepId::new(format!("step-{}", engine.max_turns)),
        status: RunStatus::MaxTurns,
        summary: format!("max_turns ({}) exhausted", engine.max_turns),
    });
    Ok(run)
}
