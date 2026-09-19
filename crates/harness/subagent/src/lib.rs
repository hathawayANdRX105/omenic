//! Subagent plugin seam for the omenic harness.
//!
//! Core types (`SubagentProvider`, `SubagentResult`, `SubagentRun`) live in
//! [`provider`]. The composition root wires a [`SubagentRuntime`] plugin that
//! stores named providers, and [`ForkProvider`] is the Phase 1 in-process
//! backend reusing `subagent::runner::run_subagent`.

pub mod acp;
pub mod fork;
pub mod provider;
pub mod runtime;
pub mod tool_subagent;
pub mod tool_subagent_control;

pub use acp::{
    AcpClient, AcpError, AcpHandlers, AcpTransport, ClientCapabilities, InitializeRequest,
    InitializeResponse, NewSessionRequest, NewSessionResponse, PROTOCOL_VERSION, PermissionOption,
    PermissionOutcome, PromptRequest, PromptResponse, RequestPermissionRequest,
    RequestPermissionResponse, StopReason, TextContent,
};
pub use fork::ForkProvider;
pub use provider::{
    SubagentCapabilities, SubagentProvider, SubagentResult, SubagentRun, SubagentStartRequest,
};
pub use runtime::{SubagentRuntime, SubagentRuntimeService};
pub use tool_subagent::ToolSubagentPlugin;
pub use tool_subagent_control::ToolSubagentControlPlugin;
