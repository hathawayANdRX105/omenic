//! Model-facing `subagent_control` tool plugin.
//!
//! Registers a `subagent_control` tool into the harness `ToolCatalog`.
//! Reference: `dsh packages/subagent/tool-subagent-control`. Two actions:
//! `list` reports the registered providers and their capabilities, and
//! `interrupt` disposes a run that `subagent` started and has not settled on.

use std::sync::Arc;

use protocol::Tool;
use protocol::{AbortSignal, ToolError, ToolResult};
use serde_json::Value;

use crate::runtime::SubagentRuntimeService;

/// Plugin that installs the `subagent_control` model-facing tool.
pub struct ToolSubagentControlPlugin {
    /// Name the tool is registered under in the catalog.
    pub tool_name: String,
}

impl Default for ToolSubagentControlPlugin {
    fn default() -> Self {
        Self {
            tool_name: "subagent_control".into(),
        }
    }
}

/// The `subagent_control` tool the model calls.
///
/// `list` is a snapshot of the provider registry; `interrupt` tears down a
/// run whose id came back from the `subagent` tool. Interrupting an unknown
/// or already-settled run is reported in the payload (`interrupted: false`),
/// not as a tool error — a control query failing is information, not a
/// broken call.
pub struct SubagentControlTool {
    name: String,
    runtime: Arc<SubagentRuntimeService>,
}

impl SubagentControlTool {
    pub fn new(name: String, runtime: Arc<SubagentRuntimeService>) -> Self {
        Self { name, runtime }
    }
}

impl Tool for SubagentControlTool {
    fn spec(&self) -> protocol::ToolSpec {
        protocol::ToolSpec {
            name: self.name.clone(),
            description: "List subagent providers, or interrupt a running subagent by run id."
                .into(),
            params_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list", "interrupt"],
                        "description": "Control action: list providers, or interrupt a run."
                    },
                    "run_id": {
                        "type": "string",
                        "description": "Required for `interrupt`: the run id returned by the subagent tool."
                    }
                },
                "required": ["action"]
            }),
        }
    }

    fn execute(&self, args: &Value, _abort: &AbortSignal) -> Result<ToolResult, ToolError> {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::Execute("missing string argument: action".into()))?;

        match action {
            "list" => {
                // `SubagentCapabilities` is not `Serialize`; shape the
                // model-facing JSON by hand so the output stays stable across
                // backends. Providers listed by `runtime.providers()` but no
                // longer resolvable by `get` are skipped.
                let mut providers = Vec::new();
                for name in self.runtime.providers() {
                    if let Some(provider) = self.runtime.get(&name) {
                        let caps = provider.capabilities();
                        providers.push(serde_json::json!({
                            "name": name,
                            "capabilities": {
                                "output_schema": caps.output_schema,
                                "depth_limit": caps.depth_limit,
                                "tool_filter": caps.tool_filter,
                                "persona": caps.persona,
                            }
                        }));
                    }
                }

                let payload = serde_json::json!({ "action": "list", "providers": providers });
                Ok(ToolResult {
                    output: serde_json::to_string(&payload)
                        .unwrap_or_else(|_| format!("{payload:?}")),
                    is_error: false,
                })
            }
            "interrupt" => {
                let run_id = args
                    .get("run_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ToolError::Execute("missing string argument: run_id".into()))?;
                let payload = if self.runtime.interrupt(run_id) {
                    serde_json::json!({ "interrupted": true, "run_id": run_id })
                } else {
                    serde_json::json!({ "interrupted": false, "run_id": run_id, "error": "no such run" })
                };
                Ok(ToolResult {
                    output: serde_json::to_string(&payload)
                        .unwrap_or_else(|_| format!("{payload:?}")),
                    is_error: false,
                })
            }
            other => Err(ToolError::Execute(format!(
                "unsupported subagent control action: {other} (supported: list, interrupt)"
            ))),
        }
    }
}
