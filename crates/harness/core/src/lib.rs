//! Shared types for the omenic agent harness.
//!
//! Aligns with `omenic agent/orbit` event semantics (`TurnStop`, `AgentEvent`)
//! and `dsh core/session` `SessionStore` lifecycle. These types are the
//! common vocabulary for every crate in `crates/harness/`.

pub mod chat;

use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Terminal status of a run, aligned with orbit `TurnStop`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    EndTurn,
    MaxTokens,
    Aborted,
    Error,
    MaxTurns,
}

/// Opaque run identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RunId(String);

impl RunId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for RunId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Opaque step identifier within a run.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StepId(String);

impl StepId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Cooperative abort signal. Reference: `omenic agent/orbit` uses
/// `AtomicBool` directly; this wrapper adds a `notify` hook for
/// `dsh`-style event-driven cancellation.
#[derive(Debug, Clone, Default)]
pub struct AbortSignal {
    /// Shared flag.
    flag: Arc<AtomicBool>,
}

impl AbortSignal {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn abort(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }
    pub fn is_aborted(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
    /// Shared flag clone: lets adapters hand the same `AtomicBool` to
    /// `omenic tools::Tool::execute`, which polls it directly.
    pub fn flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.flag)
    }

    /// Wrap an existing shared flag — the inverse of [`Self::flag`]. Lets a
    /// host adapter hand the loop's own `AtomicBool` to a harness tool
    /// instead of a fresh, unconnected flag.
    pub fn from_flag(flag: Arc<AtomicBool>) -> Self {
        Self { flag }
    }
}

/// A single LLM conversation message. Mirrors `adaptor::Message` shape
/// without the omenic dependency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

/// One unit of progress in a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub id: StepId,
    pub status: RunStatus,
    pub summary: String,
}

/// Immutable handle to a completed or in-progress run.
#[derive(Debug, Clone)]
pub struct Run {
    pub id: RunId,
    pub steps: Vec<Step>,
}

/// Read-only view over a run's state.
pub trait RunState {
    fn status(&self) -> RunStatus;
    fn steps(&self) -> &[Step];
}

impl RunState for Run {
    fn status(&self) -> RunStatus {
        self.steps
            .last()
            .map(|s| s.status)
            .unwrap_or(RunStatus::EndTurn)
    }
    fn steps(&self) -> &[Step] {
        &self.steps
    }
}

/// Errors that can terminate a run.
#[derive(Debug)]
pub enum RunError {
    Aborted,
    Llm(String),
    Tool(String),
}

impl Display for RunError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Aborted => write!(f, "run aborted"),
            RunError::Llm(msg) => write!(f, "llm error: {msg}"),
            RunError::Tool(msg) => write!(f, "tool error: {msg}"),
        }
    }
}

impl std::error::Error for RunError {}

/// LLM transport error.
#[derive(Debug, Clone)]
pub enum LlmError {
    Transport(String),
    InvalidResponse(String),
}

impl Display for LlmError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::Transport(msg) => write!(f, "transport: {msg}"),
            LlmError::InvalidResponse(msg) => write!(f, "invalid response: {msg}"),
        }
    }
}

impl std::error::Error for LlmError {}

/// Tool spec: API-facing definition. Mirrors `dsh ToolDefinition:222`
/// and `omenic adaptor::ToolDef`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub params_schema: serde_json::Value,
}

/// Tool execution result. Mirrors `dsh ToolResult:291` (simplified).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
}

/// Tool execution error.
#[derive(Debug, Clone)]
pub enum ToolError {
    Aborted,
    Execute(String),
}

impl Display for ToolError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolError::Aborted => write!(f, "tool call aborted"),
            ToolError::Execute(msg) => write!(f, "tool error: {msg}"),
        }
    }
}

impl std::error::Error for ToolError {}

/// Create a new empty run.
///
/// Reference: `omenic agent/orbit/src/lib.rs:617` (`run_agent` wrapper)
/// and `dsh core/session/src/index.ts:792` (`SessionStore.create`).
/// Constraint: `Run` must start with zero steps; `RunStatus` is set by
/// the first `run_agent_loop` call.
/// Non-goal: no persistence here; `SessionStore` handles that separately.
pub fn new_run(run_id: RunId) -> Run {
    // Reference: orbit `run_agent` starts from an empty context; status is
    // derived from the last step (`RunState::status`), so a fresh run
    // reports `EndTurn` with zero steps.
    Run {
        id: run_id,
        steps: Vec::new(),
    }
}
