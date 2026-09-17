//! Model-facing `subagent` tool plugin.
//!
//! Registers a `subagent` tool into the harness `ToolCatalog`. The tool
//! delegates to a configured `SubagentProvider` from `harness.subagents`.

use std::sync::Arc;

use omenic_harness_core::{AbortSignal, ToolError, ToolResult};
use omenic_harness_plugin::PluginContext;
use omenic_harness_tools::{Tool, ToolCatalog};
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

impl omenic_harness_plugin::DshPlugin for ToolSubagentPlugin {
    fn name(&self) -> &str {
        "tool-subagent"
    }

    fn register(&self, ctx: &mut PluginContext<'_>) {
        let runtime = ctx
            .resolve::<SubagentRuntimeService>("harness.subagents")
            .expect("SubagentRuntime must register before tool-subagent");
        let catalog = ctx
            .resolve::<ToolCatalog>("harness.tools")
            .expect("harness.tools must be provided by composition");
        catalog.register(Arc::new(SubagentTool::new(
            self.tool_name.clone(),
            self.provider_name.clone(),
            runtime,
        )));
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
    fn spec(&self) -> omenic_harness_core::ToolSpec {
        omenic_harness_core::ToolSpec {
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

        let provider = self.runtime.get(provider_name).ok_or_else(|| {
            ToolError::Execute(format!("unknown subagent provider: {provider_name}"))
        })?;

        let signal = abort.flag();
        let request = SubagentStartRequest {
            prompt: prompt.into(),
            signal,
            inherits_parent_context: false,
        };

        let run = provider.start(request);
        let result = run.result();

        // `SubagentResult` is not `Serialize`; shape the model-facing JSON
        // by hand so the output stays stable across backends.
        let payload = match &result {
            SubagentResult::Completed { output } => {
                serde_json::json!({ "status": "completed", "output": output })
            }
            SubagentResult::Failed { error } => {
                serde_json::json!({ "status": "failed", "error": error })
            }
            SubagentResult::Aborted => serde_json::json!({ "status": "aborted" }),
        };

        Ok(ToolResult {
            output: serde_json::to_string(&payload).unwrap_or_else(|_| format!("{result:?}")),
            is_error: matches!(result, SubagentResult::Failed { .. }),
        })
    }
}
