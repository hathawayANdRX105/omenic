//! B1/T3 — MCP tools reach the orbit engine through
//! [`rpc::worker::OrbitConfig::mcp_tools`] and are merged into the per-engine
//! tool list by [`rpc::worker::combined_tools`].
//!
//! The daemon owns the live MCP connections (`Arc<dyn tools::Tool>` handles
//! brought up once at start); every engine spawn gets a fresh
//! `Box<dyn tools::Tool>` shim per shared handle. These tests pin the merge
//! contract without a daemon: catalog tools stay first, MCP shims append,
//! the engine's abort signal reaches the shared tool, an empty MCP list is
//! exactly the pre-MCP behavior, and `OrbitSetup` (cloned per worker
//! respawn) stays `Clone` while sharing one tool list.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use omenic_harness_core::{AbortSignal, ToolError, ToolResult, ToolSpec};
use omenic_harness_tools::{Tool, ToolCatalog};
use orbit::LlmBackend;
use rpc::worker::{OrbitConfig, OrbitSetup, combined_tools};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Fakes: one tool per side of the seam
// ---------------------------------------------------------------------------

/// Minimal harness-side tool registered into the catalog (the "native" side).
struct FakeHarnessTool;

impl Tool for FakeHarnessTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "harness_native".into(),
            description: "fake catalog tool".into(),
            params_schema: json!({"type": "object", "properties": {}}),
        }
    }
    fn execute(&self, _args: &Value, _abort: &AbortSignal) -> Result<ToolResult, ToolError> {
        Ok(ToolResult {
            output: "harness-ok".into(),
            is_error: false,
        })
    }
}

/// Minimal agent-domain tool standing in for an `mcp::McpTool`: records
/// whether it observed an asserted abort signal, exactly as a real MCP tool
/// would (the transport polls the signal inside the round trip).
struct FakeMcpTool {
    name: String,
    seen_signal: std::sync::Mutex<bool>,
}

impl FakeMcpTool {
    fn new(name: &str) -> Self {
        FakeMcpTool {
            name: name.into(),
            seen_signal: std::sync::Mutex::new(false),
        }
    }

    fn saw_signal(&self) -> bool {
        *self.seen_signal.lock().unwrap()
    }
}

impl tools::Tool for FakeMcpTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> String {
        "fake mcp tool".into()
    }
    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }
    fn execute(&self, _args: &Value, signal: &AtomicBool) -> Result<String, tools::ToolError> {
        if signal.load(Ordering::SeqCst) {
            *self.seen_signal.lock().unwrap() = true;
        }
        Ok("mcp-ok".into())
    }
}

/// Placeholder backend: only construction/clone is exercised, never a run.
struct NoBackend;

impl LlmBackend for NoBackend {
    fn stream_cb(
        &self,
        _model: &adaptor::Model,
        _context: &adaptor::Context,
        _tools: &[adaptor::ToolDef],
        _signal: &AtomicBool,
        _emit: &mut dyn FnMut(&adaptor::StreamEvent),
    ) {
        unreachable!("construction/clone test never runs the loop");
    }
}

fn model() -> adaptor::Model {
    adaptor::Model {
        api_key: "k".into(),
        model: "test".into(),
        base_url: None,
        max_tokens: None,
    }
}

fn names_of(tools: &[Box<dyn tools::Tool>]) -> Vec<String> {
    tools.iter().map(|t| t.name().to_string()).collect()
}

// ---------------------------------------------------------------------------
// 1. Merge order: catalog first, MCP shims appended
// ---------------------------------------------------------------------------

#[test]
fn combined_tools_appends_mcp_shims_after_catalog_tools() {
    let catalog = ToolCatalog::new();
    catalog.register(Arc::new(FakeHarnessTool));
    let alpha = Arc::new(FakeMcpTool::new("mcp__fake__alpha"));
    let beta = Arc::new(FakeMcpTool::new("mcp__fake__beta"));
    let mcp_tools: Arc<Vec<Arc<dyn tools::Tool>>> = Arc::new(vec![
        Arc::clone(&alpha) as Arc<dyn tools::Tool>,
        Arc::clone(&beta) as Arc<dyn tools::Tool>,
    ]);

    let flag = Arc::new(AtomicBool::new(false));
    let merged = combined_tools(&catalog, &mcp_tools, Arc::clone(&flag));
    assert_eq!(
        names_of(&merged),
        vec![
            "harness_native".to_string(),
            "mcp__fake__alpha".to_string(),
            "mcp__fake__beta".to_string(),
        ],
        "catalog tools stay first, one shim per shared MCP tool appended"
    );

    // Zero-diff guarantee: an empty MCP list is exactly the pre-MCP result.
    let empty: Arc<Vec<Arc<dyn tools::Tool>>> = Arc::new(Vec::new());
    let merged = combined_tools(&catalog, &empty, Arc::clone(&flag));
    assert_eq!(names_of(&merged), vec!["harness_native".to_string()]);
}

// ---------------------------------------------------------------------------
// 2. Shim delegation: spec + abort signal reach the shared tool
// ---------------------------------------------------------------------------

#[test]
fn mcp_shim_forwards_spec_and_engine_abort_to_the_shared_tool() {
    let catalog = ToolCatalog::new();
    let alpha = Arc::new(FakeMcpTool::new("mcp__fake__alpha"));
    let mcp_tools: Arc<Vec<Arc<dyn tools::Tool>>> =
        Arc::new(vec![Arc::clone(&alpha) as Arc<dyn tools::Tool>]);

    // The engine's abort flag: what the orbit loop passes into tool execute.
    let abort = Arc::new(AtomicBool::new(false));
    let merged = combined_tools(&catalog, &mcp_tools, Arc::clone(&abort));
    let shim = merged
        .iter()
        .find(|t| t.name() == "mcp__fake__alpha")
        .expect("shim present in the merged list");

    assert_eq!(shim.description(), "fake mcp tool");
    assert_eq!(
        shim.parameters(),
        json!({"type": "object", "properties": {}})
    );

    // Signal NOT set: the shared tool sees a clean flag.
    let out = shim
        .execute(&json!({"x": 1}), &abort)
        .expect("shim execute delegates");
    assert_eq!(out, "mcp-ok");
    assert!(!alpha.saw_signal(), "clean flag must not read as aborted");

    // Engine aborts, then the loop dispatches: the forwarded flag is the
    // same one the shared tool polls — an abort reaches an in-flight MCP
    // round trip.
    abort.store(true, Ordering::SeqCst);
    let _ = shim.execute(&json!({}), &abort);
    assert!(
        alpha.saw_signal(),
        "the engine's abort flag must reach the shared MCP tool"
    );
}

// ---------------------------------------------------------------------------
// 3. OrbitSetup stays Clone and shares one MCP tool list
// ---------------------------------------------------------------------------

#[test]
fn orbit_setup_with_mcp_tools_stays_clone_and_shares_the_list() {
    let mcp_tools: Arc<Vec<Arc<dyn tools::Tool>>> =
        Arc::new(vec![
            Arc::new(FakeMcpTool::new("mcp__fake__solo")) as Arc<dyn tools::Tool>
        ]);
    let setup = OrbitSetup {
        model: model(),
        backend: Arc::new(NoBackend),
        config: OrbitConfig {
            cwd: None,
            max_turns: 8,
            compaction: Arc::new(omenic_harness_compaction::CharBudgetPolicy::default()),
            catalog: Arc::new(ToolCatalog::new()),
            mcp_tools: Arc::clone(&mcp_tools),
        },
        providers: Vec::new(),
    };

    // Load-bearing for worker (re)spawn: the daemon clones the setup per
    // engine build. The clone must stay cheap and share the tool list.
    let cloned = setup.clone();
    assert!(
        Arc::ptr_eq(&setup.config.mcp_tools, &cloned.config.mcp_tools),
        "clone must share the MCP tool list, not deep-copy it"
    );

    let flag = Arc::new(AtomicBool::new(false));
    let merged = combined_tools(
        &cloned.config.catalog,
        &cloned.config.mcp_tools,
        Arc::clone(&flag),
    );
    assert_eq!(names_of(&merged), vec!["mcp__fake__solo".to_string()]);
}
