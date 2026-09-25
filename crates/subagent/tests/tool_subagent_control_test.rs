use std::sync::Arc;

use plugin::{DshPlugin, Fiber};
use protocol::AbortSignal;
use subagent::{
    SubagentCapabilities, SubagentProvider, SubagentRun, SubagentRuntime, SubagentRuntimeService,
    SubagentStartRequest, ToolSubagentControlPlugin,
};
use tools::{ToolCatalog, ToolExecutor};

/// Mock provider with a fixed name and all-bool capabilities.
struct MockProvider;

impl SubagentProvider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }
    fn capabilities(&self) -> SubagentCapabilities {
        SubagentCapabilities::default()
    }
    fn inherits_parent_context(&self) -> bool {
        false
    }
    fn start(&self, _request: SubagentStartRequest) -> SubagentRun {
        unreachable!("control tool never starts runs");
    }
}

#[test]
fn tool_subagent_control_registers_into_catalog() {
    let mut fiber = Fiber::default();
    {
        let mut ctx = fiber.context();
        ctx.provide("harness.tools", ToolCatalog::new());
        SubagentRuntime::default().register(&mut ctx);
        ToolSubagentControlPlugin::default().register(&mut ctx);
    }
    let catalog: Arc<ToolCatalog> = fiber.resolve("harness.tools").unwrap();
    let specs = catalog.specs();
    assert!(
        specs.iter().any(|s| s.name == "subagent_control"),
        "subagent_control tool must be registered"
    );
}

#[test]
fn tool_subagent_control_list_reports_registered_provider() {
    let mut fiber = Fiber::default();
    {
        let mut ctx = fiber.context();
        ctx.provide("harness.tools", ToolCatalog::new());
        SubagentRuntime::default().register(&mut ctx);
        ToolSubagentControlPlugin::default().register(&mut ctx);
    }
    let runtime: Arc<SubagentRuntimeService> = fiber.resolve("harness.subagents").unwrap();
    runtime.register("mock", Arc::new(MockProvider));

    let catalog: Arc<ToolCatalog> = fiber.resolve("harness.tools").unwrap();
    let spec = catalog
        .specs()
        .into_iter()
        .find(|s| s.name == "subagent_control")
        .unwrap();
    let out = catalog
        .execute(
            &spec,
            &serde_json::json!({"action": "list"}),
            &AbortSignal::new(),
        )
        .unwrap();
    assert!(!out.is_error);
    assert!(
        out.output.contains("\"mock\""),
        "list output must name the provider, got: {}",
        out.output
    );
    assert!(
        out.output.contains("providers"),
        "list output must carry a providers array, got: {}",
        out.output
    );
}

#[test]
fn tool_subagent_control_rejects_unsupported_action() {
    let mut fiber = Fiber::default();
    {
        let mut ctx = fiber.context();
        ctx.provide("harness.tools", ToolCatalog::new());
        SubagentRuntime::default().register(&mut ctx);
        ToolSubagentControlPlugin::default().register(&mut ctx);
    }
    let catalog: Arc<ToolCatalog> = fiber.resolve("harness.tools").unwrap();
    let spec = catalog
        .specs()
        .into_iter()
        .find(|s| s.name == "subagent_control")
        .unwrap();
    // `interrupt` is a supported action as of Phase 4; the unsupported-action
    // contract is exercised by an action the tool genuinely does not know.
    let err = catalog
        .execute(
            &spec,
            &serde_json::json!({"action": "bogus"}),
            &AbortSignal::new(),
        )
        .err()
        .expect("unsupported action must be a tool error, not a fake success");
    assert!(
        err.to_string()
            .contains("unsupported subagent control action: bogus"),
        "error must name the rejected action, got: {err}"
    );
}

#[test]
fn tool_subagent_control_list_with_zero_providers_is_valid_json() {
    let mut fiber = Fiber::default();
    {
        let mut ctx = fiber.context();
        ctx.provide("harness.tools", ToolCatalog::new());
        SubagentRuntime::default().register(&mut ctx);
        ToolSubagentControlPlugin::default().register(&mut ctx);
    }
    let catalog: Arc<ToolCatalog> = fiber.resolve("harness.tools").unwrap();
    let spec = catalog
        .specs()
        .into_iter()
        .find(|s| s.name == "subagent_control")
        .unwrap();
    let out = catalog
        .execute(
            &spec,
            &serde_json::json!({"action": "list"}),
            &AbortSignal::new(),
        )
        .unwrap();
    assert!(!out.is_error);
    let parsed: serde_json::Value =
        serde_json::from_str(&out.output).expect("list output must be valid JSON");
    assert_eq!(parsed["action"], "list");
    assert_eq!(parsed["providers"], serde_json::json!([]));
}
