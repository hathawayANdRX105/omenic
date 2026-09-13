//! Runtime engine for the omenic agent harness.
//!
//! Reference: `omenic agent/orbit/src/lib.rs:617` (`run_agent`) and
//! `dsh agent-loop`.
//!
//! This crate abstracts the provider call loop and tool dispatch so
//! that concrete implementations (OMP, deepseek-harness, etc.) can
//! be swapped in without changing the invariant loop.

use omenic_harness_core::{
    AbortSignal, LlmError, Message, Run, RunError, RunId, RunStatus, Step, ToolResult, ToolSpec,
};
use omenic_harness_tools::{ToolCatalog, ToolExecutor};
use std::pin::Pin;

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
    // future fields (maintain, get_steering, etc.) added later
}

/// Runs the full agent loop until `RunStatus::EndTurn` or an error.
///
/// Reference: `omenic agent/orbit/src/lib.rs:617` (`run_agent`) and
/// `dsh agent-loop` core transaction.
/// Constraint: must call provider, execute tools via `executor`,
/// respect `abort`, and return a complete `Run` with all steps.
/// Non-goal: no streaming, no compaction policy, no session persistence.
pub fn run_agent_loop(
    engine: &LoopEngine,
    provider: &impl Provider,
    executor: &impl ToolExecutor,
    abort: &AbortSignal,
) -> Result<Run, RunError> {
    todo!(
        "TODO(#TBD): run_agent_loop — reference: omenic/crates/agent/orbit/src/lib.rs:617; \
         constraint: full loop that returns Run with steps; \
         non-goal: no streaming, no compaction"
    )
}
