//! Tool execution layer for the omenic agent harness.
//!
//! Reference: `dsh core/tools/src/index.ts:222` (`ToolDefinition`) and
//! `omenic agent/tools/src/lib.rs:105` (`Tool` trait).
//!
//! This crate defines the `ToolExecutor` trait and `ToolCatalog` for
//! registering and dispatching tool calls. Tool specs live in
//! `omenic-harness-core`.

use omenic_harness_core::{AbortSignal, ToolError, ToolResult, ToolSpec};
use serde_json::Value;
use std::sync::Arc;

/// A registered tool with its own spec and execution logic.
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    fn execute(&self, args: &Value, abort: &AbortSignal) -> Result<ToolResult, ToolError>;
}

/// Registry of available tools, keyed by name.
pub struct ToolCatalog {
    tools: Vec<Arc<dyn Tool>>,
}

impl ToolCatalog {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.push(tool);
    }

    /// Look up a tool by name.
    pub fn find(&self, name: &str) -> Option<&dyn Tool> {
        self.tools
            .iter()
            .find(|t| t.spec().name == name)
            .map(|t| t.as_ref() as &dyn Tool)
    }

    /// All specs in registration order.
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.iter().map(|t| t.spec()).collect()
    }
}

impl Default for ToolCatalog {
    fn default() -> Self {
        Self::new()
    }
}

/// Executes tool calls against a catalog.
///
/// Reference: `dsh ToolDefinition.execute:235` and
/// `omenic agent/tools/src/lib.rs:111` (`Tool::execute`).
/// Constraint: synchronous; `abort` is polled by long-running tools to
/// support cooperative cancellation.
/// Non-goal: no parallel tool dispatch; no schema validation here.
pub trait ToolExecutor {
    fn execute(
        &self,
        spec: &ToolSpec,
        args: &Value,
        abort: &AbortSignal,
    ) -> Result<ToolResult, ToolError>;
}

/// Default catalog built from the omenic built-in tools.
///
/// Reference: `omenic agent/tools/src/lib.rs:319` (`builtin_tools`).
/// Constraint: returns a `ToolCatalog` with one entry per built-in tool.
/// Non-goal: no MCP tools here; no policy wrapping
/// (`Guarded`/`Policy` is omenic-specific, not part of the harness contract).
pub fn default_catalog() -> ToolCatalog {
    todo!(
        "TODO(#TBD): default_catalog — reference: omenic/crates/agent/tools/src/lib.rs:319; \
         constraint: one ToolCatalog entry per built-in tool; \
         non-goal: no MCP tools, no Policy wrapping"
    )
}
