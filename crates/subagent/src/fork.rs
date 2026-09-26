use std::sync::mpsc;
use std::thread;

use crate::provider::{SubagentProvider, SubagentResult, SubagentStartRequest};

/// In-process fork backend: spawns a thread that calls
/// `crate::runner::run_subagent` with a read-only tool set and reports the
/// terminal outcome through an `mpsc` channel.
///
/// For Phase 1 the child always starts fresh (no parent prefix seed); the
/// `inherits_parent_context` flag is descriptive only and will drive
/// session-seeding in Phase 2.
pub struct ForkProvider {
    backend: std::sync::Arc<dyn agent_loop::orbit::LlmBackend + Send + Sync>,
    model: llm::Model,
    tools: std::sync::Arc<Vec<Box<dyn tools::Tool>>>,
    max_turns: usize,
}

impl ForkProvider {
    /// Build a fork provider with explicit backend, model, tool set, and
    /// per-run turn cap.
    pub fn new(
        backend: std::sync::Arc<dyn agent_loop::orbit::LlmBackend + Send + Sync>,
        model: llm::Model,
        tools: std::sync::Arc<Vec<Box<dyn tools::Tool>>>,
        max_turns: usize,
    ) -> Self {
        Self {
            backend,
            model,
            tools,
            max_turns,
        }
    }
}

impl SubagentProvider for ForkProvider {
    fn name(&self) -> &str {
        "fork"
    }

    fn capabilities(&self) -> crate::provider::SubagentCapabilities {
        // Phase 1: the request carries no per-run knobs yet; report nothing
        // supported until depth/tool_filter/persona actually reach the runner.
        crate::provider::SubagentCapabilities::default()
    }

    fn inherits_parent_context(&self) -> bool {
        // The fork runner starts with `context_seed: None`; parent context
        // seeding is a Phase 2 feature.
        false
    }

    fn supports_mid_run_messages(&self) -> bool {
        // The fork backend runs the in-process loop, which drains the inbox
        // as steering at every step boundary.
        true
    }

    fn start(&self, request: SubagentStartRequest) -> crate::provider::SubagentRun {
        let tools = self.tools.clone();
        let backend = self.backend.clone();
        let model = self.model.clone();
        let signal = request.signal.clone();
        let prompt = request.prompt;
        let max_turns = self.max_turns;
        let inbox = request.inbox.clone();
        let (tx, rx) = mpsc::channel();

        let join = thread::spawn(move || {
            let outcome = crate::runner::run_subagent(
                backend.as_ref(),
                &model,
                &prompt,
                max_turns,
                &*tools,
                &signal,
                0,
                None,
                inbox.as_deref(),
            );
            let result = match outcome {
                Ok(output) => SubagentResult::Completed { output },
                Err(crate::SubagentError::Aborted { .. }) => SubagentResult::Aborted,
                Err(e) => SubagentResult::Failed {
                    error: e.to_string(),
                },
            };
            let _ = tx.send(result);
        });

        crate::provider::SubagentRun::new(
            rx,
            join,
            std::sync::Arc::new(ForkDisposer {
                signal: request.signal,
            }),
        )
    }
}

/// In-process disposer: the worker polls `signal` between turns.
struct ForkDisposer {
    signal: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl crate::provider::RunDisposer for ForkDisposer {
    fn dispose(&self) {
        self.signal
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
