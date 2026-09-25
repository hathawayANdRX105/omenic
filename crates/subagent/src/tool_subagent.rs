//! Model-facing `subagent` tool plugin.
//!
//! Registers a `subagent` tool into the harness `ToolCatalog`. The tool
//! delegates to a configured `SubagentProvider` from `harness.subagents`.

use std::sync::Arc;

use protocol::Tool;
use protocol::{AbortSignal, ToolError, ToolResult};
use serde_json::Value;

use crate::provider::{SubagentResult, SubagentStartRequest};
use crate::runtime::SubagentRuntimeService;

/// Plugin that installs the `subagent` model-facing tool.
pub struct ToolSubagentPlugin {
    /// Provider the tool falls back to when the model omits `provider`.
    pub provider_name: String,
    /// Name the tool is registered under in the catalog.
    pub tool_name: String,
}

impl Default for ToolSubagentPlugin {
    fn default() -> Self {
        Self {
            provider_name: "fork".into(),
            tool_name: "subagent".into(),
        }
    }
}

/// The `subagent` tool the model calls.
pub struct SubagentTool {
    name: String,
    provider_name: String,
    runtime: Arc<SubagentRuntimeService>,
}

impl SubagentTool {
    pub fn new(name: String, provider_name: String, runtime: Arc<SubagentRuntimeService>) -> Self {
        Self {
            name,
            provider_name,
            runtime,
        }
    }
}

impl Tool for SubagentTool {
    fn spec(&self) -> protocol::ToolSpec {
        protocol::ToolSpec {
            name: self.name.clone(),
            description:
                "Delegate a prompt to a subagent provider and return its collected output.".into(),
            params_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "The prompt to send to the subagent."
                    },
                    "provider": {
                        "type": "string",
                        "description": "Provider name to use (default: fork)."
                    }
                },
                "required": ["prompt"]
            }),
        }
    }

    fn execute(&self, args: &Value, abort: &AbortSignal) -> Result<ToolResult, ToolError> {
        let prompt = args
            .get("prompt")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::Execute("missing string argument: prompt".into()))?;
        let provider_name = args
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or(&self.provider_name);

        let signal = abort.flag();
        let request = SubagentStartRequest {
            prompt: prompt.into(),
            signal,
            inherits_parent_context: false,
        };

        let (run_id, run) = self
            .runtime
            .start_run(provider_name, request)
            .map_err(ToolError::Execute)?;
        let result = run.result();
        // The run has settled, win or lose — retire it so a later
        // `interrupt` cannot target a handle whose worker is already gone.
        self.runtime.finish_run(&run_id);

        // `SubagentResult` is not `Serialize`; shape the model-facing JSON
        // by hand so the output stays stable across backends. `run_id` lets
        // the model follow up with `subagent_control` while the run is live.
        let payload = match &result {
            SubagentResult::Completed { output } => {
                serde_json::json!({ "status": "completed", "output": output, "run_id": run_id })
            }
            SubagentResult::Failed { error } => {
                serde_json::json!({ "status": "failed", "error": error, "run_id": run_id })
            }
            SubagentResult::Aborted => {
                serde_json::json!({ "status": "aborted", "run_id": run_id })
            }
        };

        Ok(ToolResult {
            output: serde_json::to_string(&payload).unwrap_or_else(|_| format!("{result:?}")),
            is_error: matches!(result, SubagentResult::Failed { .. }),
        })
    }
}
