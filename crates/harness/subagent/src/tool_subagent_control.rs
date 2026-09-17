//! Model-facing `subagent_control` tool plugin.
//!
//! Registers a `subagent_control` tool into the harness `ToolCatalog`.
//! Reference: `dsh packages/subagent/tool-subagent-control`. Phase 1 exposes
//! a single `list` action that reports the registered providers and their
//! capabilities; interrupt lands with the out-of-process backends (Phase 4)
//! once runs can be disposed from outside the one-shot blocking `execute`.

use std::sync::Arc;

use omenic_harness_core::{AbortSignal, ToolError, ToolResult};
use omenic_harness_plugin::PluginContext;
use omenic_harness_tools::{Tool, ToolCatalog};
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

impl omenic_harness_plugin::DshPlugin for ToolSubagentControlPlugin {
    fn name(&self) -> &str {
        "tool-subagent-control"
    }

    fn register(&self, ctx: &mut PluginContext<'_>) {
        let runtime = ctx
            .resolve::<SubagentRuntimeService>("harness.subagents")
            .expect("SubagentRuntime must register before tool-subagent-control");
        let catalog = ctx
            .resolve::<ToolCatalog>("harness.tools")
            .expect("harness.tools must be provided by composition");
        catalog.register(Arc::new(SubagentControlTool::new(
            self.tool_name.clone(),
            runtime,
        )));
    }
}

/// The `subagent_control` tool the model calls.
///
/// Phase 1 is one-shot: `execute` blocks on the listing and never cancels
/// anything. Interrupt support lands in Phase 4 with the out-of-process
/// backends (ACP/Codex/Claude Code/SDK), where runs are long-lived and
/// disposable from the outside.
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
    fn spec(&self) -> omenic_harness_core::ToolSpec {
        omenic_harness_core::ToolSpec {
            name: self.name.clone(),
            description: "List available subagent providers and their capabilities.".into(),
            params_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list"],
                        "description": "Control action (Phase 1: list only)."
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

        if action != "list" {
            // Honest refusal: interrupt is not a Phase 1 action, so an
            // unsupported value is an error, never a fake success.
            return Err(ToolError::Execute(format!(
                "unsupported subagent control action: {action} (Phase 1 supports only list)"
            )));
        }

        // `SubagentCapabilities` is not `Serialize`; shape the model-facing
        // JSON by hand so the output stays stable across backends. Providers
        // listed by `runtime.providers()` but no longer resolvable by
        // `get` are skipped.
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
            output: serde_json::to_string(&payload).unwrap_or_else(|_| format!("{payload:?}")),
            is_error: false,
        })
    }
}
