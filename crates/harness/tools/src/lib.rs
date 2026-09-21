//! Tool execution layer for the omenic agent harness.
//!
//! Reference: `dsh core/tools/src/index.ts:222` (`ToolDefinition`) and
//! `omenic agent/tools/src/lib.rs:105` (`Tool` trait).
//!
//! This crate defines the `ToolExecutor` trait and `ToolCatalog` for
//! registering and dispatching tool calls. Tool specs live in
//! `omenic-harness-core`.

pub mod jobs_terminal;
pub mod web;

use omenic_harness_core::{AbortSignal, ToolError, ToolResult, ToolSpec};
use serde_json::Value;
use std::sync::{Arc, Mutex};

/// A registered tool with its own spec and execution logic.
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    fn execute(&self, args: &Value, abort: &AbortSignal) -> Result<ToolResult, ToolError>;
}

/// Registry of available tools, keyed by name.
///
/// Interior mutability: plugins register tools through a shared
/// `PluginContext` during load (`register` takes `&self`), matching
/// `dsh ctx.tools.register(...)`.
pub struct ToolCatalog {
    tools: Mutex<Vec<Arc<dyn Tool>>>,
}

impl ToolCatalog {
    pub fn new() -> Self {
        Self {
            tools: Mutex::new(Vec::new()),
        }
    }

    pub fn register(&self, tool: Arc<dyn Tool>) {
        self.tools.lock().unwrap().push(tool);
    }

    /// Look up a tool by name.
    pub fn find(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools
            .lock()
            .unwrap()
            .iter()
            .find(|t| t.spec().name == name)
            .cloned()
    }

    /// All specs in registration order.
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools
            .lock()
            .unwrap()
            .iter()
            .map(|t| t.spec())
            .collect()
    }

    /// Cloned handles to every registered tool, in registration order. A
    /// host that dispatches through a different tool trait (orbit's
    /// `tools::Tool`) adapts each handle at the seam instead of re-building
    /// the catalog.
    pub fn all(&self) -> Vec<Arc<dyn Tool>> {
        self.tools.lock().unwrap().iter().cloned().collect()
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
    /// Tool specs advertised to the provider. Default: none, so
    /// implementors that expose a fixed tool set override this.
    fn specs(&self) -> Vec<ToolSpec> {
        Vec::new()
    }
    fn execute(
        &self,
        spec: &ToolSpec,
        args: &Value,
        abort: &AbortSignal,
    ) -> Result<ToolResult, ToolError>;
}

impl ToolExecutor for ToolCatalog {
    fn specs(&self) -> Vec<ToolSpec> {
        ToolCatalog::specs(self)
    }
    fn execute(
        &self,
        spec: &ToolSpec,
        args: &Value,
        abort: &AbortSignal,
    ) -> Result<ToolResult, ToolError> {
        self.find(&spec.name)
            .ok_or_else(|| ToolError::Execute(format!("unknown tool: {}", spec.name)))?
            .execute(args, abort)
    }
}

/// Adapter: omenic `tools::Tool` -> harness `Tool`.
struct Builtin(Box<dyn tools::Tool>);

impl Tool for Builtin {
    fn spec(&self) -> ToolSpec {
        let def = tools::def(self.0.as_ref());
        ToolSpec {
            name: def.name,
            description: def.description,
            params_schema: def.parameters,
        }
    }
    fn execute(&self, args: &Value, abort: &AbortSignal) -> Result<ToolResult, ToolError> {
        let flag = abort.flag();
        self.0
            .execute(args, &flag)
            .map(|output| ToolResult {
                output,
                is_error: false,
            })
            .map_err(|e| ToolError::Execute(e.to_string()))
    }
}

/// Default catalog: one entry per omenic built-in tool.
///
/// Reference: `omenic agent/tools/src/lib.rs:319` (`builtin_tools`).
/// Non-goal: no MCP tools; no `Guarded`/`Policy` wrapping (omenic-specific).
pub fn default_catalog() -> ToolCatalog {
    let catalog = ToolCatalog::new();
    for tool in tools::builtin_tools() {
        catalog.register(Arc::new(Builtin(tool)));
    }
    // Internet tools live here (not in agent/tools::builtin_tools) because
    // they are harness-side capabilities with their own SSRF/bounds policy;
    // ref: deepseek-harness-rs/src/tools/web_{fetch,search}.rs.
    catalog.register(Arc::new(web::WebFetchTool));
    catalog.register(Arc::new(web::WebSearchTool));
    catalog
}

/// Built-in tools whose names are in `wanted`, in `builtin_tools()` order.
///
/// Shared filter used by the daemon's fork subagent registration and its
/// test, so the read-only allow-list lives in one place.
pub fn filter_builtin_tools(wanted: &[&str]) -> Vec<Box<dyn tools::Tool>> {
    tools::builtin_tools()
        .into_iter()
        .filter(|t| wanted.contains(&tools::def(&**t).name.as_str()))
        .collect()
}
