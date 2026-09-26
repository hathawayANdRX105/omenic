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
                    },
                    "background": {
                        "type": "boolean",
                        "description": "Do not wait for the result. The run keeps going; its completion arrives as an aside and its output is fetched with `subagent_control` `result <run_id>`."
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
        let background = args
            .get("background")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let signal = abort.flag();
        let request = SubagentStartRequest {
            prompt: prompt.into(),
            signal,
            inherits_parent_context: false,
            inbox: None,
        };

        let (run_id, run) = self
            .runtime
            .start_run(provider_name, request)
            .map_err(ToolError::Execute)?;

        // Background: do not block. Watch for the result on a side thread and
        // let the runtime's settle hook surface it to the model as an aside.
        // `run` is `Send` (its channels and JoinHandle are), so moving it into
        // the thread is sound; the provider's worker is already on its own
        // thread.
        if background {
            let runtime = Arc::clone(&self.runtime);
            let watch_id = run_id.clone();
            std::thread::spawn(move || {
                let result = run.result();
                runtime.settle(&watch_id, &result);
            });
            let payload = serde_json::json!({
                "status": "started",
                "run_id": run_id,
                "note": "completion arrives as an aside; fetch output with subagent_control result"
            });
            return Ok(ToolResult {
                output: serde_json::to_string(&payload).unwrap_or_else(|_| format!("{payload:?}")),
                is_error: false,
            });
        }

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
