//! Subagent plugin seam for the omenic harness.
//!
//! Core types (`SubagentProvider`, `SubagentResult`, `SubagentRun`) live in
//! [`provider`]. The composition root wires a [`SubagentRuntime`] plugin that
//! stores named providers, and [`ForkProvider`] is the Phase 1 in-process
//! backend reusing `subagent::runner::run_subagent`.

pub mod fork;
pub mod provider;
pub mod runtime;
pub mod tool_subagent;

pub use fork::ForkProvider;
pub use provider::{
    SubagentCapabilities, SubagentProvider, SubagentResult, SubagentRun, SubagentStartRequest,
};
pub use runtime::{SubagentRuntime, SubagentRuntimeService};
pub use tool_subagent::ToolSubagentPlugin;
