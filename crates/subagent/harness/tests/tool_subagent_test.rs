use std::sync::Arc;

use plugin::{DshPlugin, Fiber};
use protocol::AbortSignal;
use subagent_harness::{
    RunDisposer, SubagentCapabilities, SubagentProvider, SubagentResult, SubagentRun,
    SubagentRuntime, SubagentRuntimeService, SubagentStartRequest, ToolSubagentPlugin,
};
use tools_harness::{ToolCatalog, ToolExecutor};

/// Mock provider that returns a fixed result.
struct MockProvider {
    result: SubagentResult,
}

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
        // Build a run with a pre-resolved result (channel already has result)
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = tx.send(self.result.clone());
        SubagentRun::new(
            rx,
            std::thread::spawn(|| {}),
            std::sync::Arc::new(NoopDisposer),
        )
    }
}

/// No-op disposer: the result is already resolved, nothing to tear down.
struct NoopDisposer;

impl RunDisposer for NoopDisposer {
    fn dispose(&self) {}
}

#[test]
fn tool_subagent_plugin_registers_into_catalog() {
    let mut fiber = Fiber::default();
    // SubagentRuntime first, plus the tool catalog the plugin will register
    // into (`assemble` provides it in real composition; tests do it by hand).
    {
        let mut ctx = fiber.context();
        ctx.provide("harness.tools", ToolCatalog::new());
        SubagentRuntime::default().register(&mut ctx);
    }
    // Register mock provider directly
    let runtime: Arc<SubagentRuntimeService> = fiber.resolve("harness.subagents").unwrap();
    runtime.register(
        "mock",
        Arc::new(MockProvider {
            result: SubagentResult::Completed {
                output: "ok".into(),
            },
        }),
    );
    // tool-subagent
    {
        let mut ctx = fiber.context();
        ToolSubagentPlugin::default().register(&mut ctx);
    }
    let catalog: Arc<ToolCatalog> = fiber.resolve("harness.tools").unwrap();
    let specs = catalog.specs();
    assert!(
        specs.iter().any(|s| s.name == "subagent"),
        "subagent tool must be registered"
    );
}

#[test]
fn tool_subagent_execute_returns_provider_output() {
    let mut fiber = Fiber::default();
    {
        let mut ctx = fiber.context();
        ctx.provide("harness.tools", ToolCatalog::new());
        SubagentRuntime::default().register(&mut ctx);
    }
    let runtime: Arc<SubagentRuntimeService> = fiber.resolve("harness.subagents").unwrap();
    runtime.register(
        "mock",
        Arc::new(MockProvider {
            result: SubagentResult::Completed {
                output: "hello from mock".into(),
            },
        }),
    );
    {
        let mut ctx = fiber.context();
        ToolSubagentPlugin {
            provider_name: "mock".into(),
            ..Default::default()
        }
        .register(&mut ctx);
    }
    let catalog: Arc<ToolCatalog> = fiber.resolve("harness.tools").unwrap();
    let spec = catalog
        .specs()
        .into_iter()
        .find(|s| s.name == "subagent")
        .unwrap();
    let out = catalog
        .execute(
            &spec,
            &serde_json::json!({"prompt": "hi"}),
            &AbortSignal::new(),
        )
        .unwrap();
    assert!(!out.is_error);
    assert!(
        out.output.contains("hello from mock"),
        "got: {}",
        out.output
    );
}

#[test]
fn tool_subagent_execute_rejects_unknown_provider() {
    let mut fiber = Fiber::default();
    {
        let mut ctx = fiber.context();
        ctx.provide("harness.tools", ToolCatalog::new());
        SubagentRuntime::default().register(&mut ctx);
        ToolSubagentPlugin::default().register(&mut ctx);
    }
    let catalog: Arc<ToolCatalog> = fiber.resolve("harness.tools").unwrap();
    let spec = catalog
        .specs()
        .into_iter()
        .find(|s| s.name == "subagent")
        .unwrap();
    // An unknown provider is a tool-level error (mirrors `ToolCatalog::execute`
    // for an unknown tool), not an `is_error` output.
    let err = catalog
        .execute(
            &spec,
            &serde_json::json!({"prompt": "hi", "provider": "nope"}),
            &AbortSignal::new(),
        )
        .expect_err("unknown provider must be a tool error");
    assert!(
        err.to_string().contains("unknown subagent provider: nope"),
        "error must name the rejected provider, got: {err}"
    );
}
