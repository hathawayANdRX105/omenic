// Events emitted by the agent loop, for UI/evidence consumption.
use serde::{Deserialize, Serialize};
// Turn shape mirrors oh-my-pi's AgentEvent (agent-loop.ts): per-LLM-round
// `TurnStart`, streamed text deltas, tool dispatch start/end, turn end.
// Serde shape is the frozen cross-crate event contract (R2 3.1):
// `{"type":"turn_start"}`, `{"type":"assistant_text","delta":…}`,
// `{"type":"tool_call","id":…,"name":…,"args":…}` (flattened
// `ToolCallSpec`), `{"type":"tool_start",…}`, `{"type":"tool_result",…}`,
// `{"type":"turn_end","stop_reason":…}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// One LLM round-trip begins (before the stream, after maintenance).
    TurnStart,
    AssistantText {
        delta: String,
    },
    AssistantReasoning {
        delta: String,
    },
    /// Tool call parsed from the stream (not yet executed).
    ToolCall(ToolCallSpec),
    /// Tool dispatch begins (OMP `tool_execution_start`).
    ToolStart {
        id: String,
        name: String,
    },
    ToolResult {
        id: String,
        name: String,
        result: String,
    },
    TurnEnd {
        stop_reason: TurnStop,
    },
}

/// Loop-level stop reasons (`error`/`max_turns` added on top of the stream set).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStop {
    EndTurn,
    MaxTokens,
    Aborted,
    Error,
    /// [`LoopConfig::max_turns`] LLM round-trips exhausted.
    MaxTurns,
}

/// A completed tool call extracted from the stream.
/// Serde-wise this is the payload of `protocol::AgentEvent::ToolCall`; the
/// field names are the cross-crate event contract (R2 3.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallSpec {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
}
