//! Adapter: one remote MCP tool presented as a built-in `tools::Tool`.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde_json::Value;
use tools::{Tool, ToolError};

use crate::{McpTransport, call_tool};

/// Metadata for one remote tool, as returned by `tools/list`.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolMeta {
    /// Name exposed to the model, namespaced per server (`mcp__<server>__<tool>`)
    /// so two servers — or a server and a built-in — can't shadow each other.
    pub name: String,
    /// Name the server knows, sent back verbatim in `tools/call`.
    pub remote: String,
    pub description: String,
    /// JSON Schema from the server's `inputSchema`.
    pub parameters: Value,
}

/// A remote MCP tool. `execute` is a synchronous `tools/call` round trip.
pub struct McpTool {
    meta: ToolMeta,
    transport: Arc<dyn McpTransport>,
}

impl McpTool {
    pub fn new(meta: ToolMeta, transport: Arc<dyn McpTransport>) -> McpTool {
        McpTool { meta, transport }
    }

    /// Name the server knows this tool by.
    pub fn remote(&self) -> &str {
        &self.meta.remote
    }
}

impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.meta.name
    }

    fn description(&self) -> String {
        self.meta.description.clone()
    }

    fn parameters(&self) -> Value {
        self.meta.parameters.clone()
    }

    fn execute(&self, args: &Value, signal: &AtomicBool) -> Result<String, ToolError> {
        call_tool(self.transport.as_ref(), &self.meta.remote, args, signal).map_err(ToolError::from)
    }
}
