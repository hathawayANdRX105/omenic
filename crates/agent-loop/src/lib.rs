//! Agent-loop config carrier for the omenic harness.
//!
//! Holds only [`LoopEngine`], the runtime config the composition root
//! registers under `"harness.loop"`. The live agent loop is
//! `agent_loop::orbit::run_agent_streaming` (streaming, tool execution, compaction
//! bridge); this crate deliberately has no loop of its own, so there is a
//! single main-loop implementation. `daemon/rpc/worker.rs` reads
//! `max_turns` and `model` from the resolved engine to drive that loop.
//!
//! Reference: `dsh agent-loop`. The parallel non-streaming loop that once
//! lived here (`run_agent_loop` + `Provider`) was a dead second main loop
//! and was removed to keep the loop single-sourced.

pub mod compaction;
pub mod instruction;
pub mod orbit;
/// Central loop engine holding runtime config (max turns, model).
///
/// Config carrier only: no behavior. `daemon` resolves it from the
/// `"harness.loop"` service and passes `max_turns` to the turn cap and
#[derive(Debug, Default)]
pub struct LoopEngine {
    /// Hard cap on LLM round-trips per run.
    pub max_turns: usize,
    /// Model name forwarded to the live loop.
    pub model: String,
    // future fields (maintain, get_steering, etc.) added later
}
