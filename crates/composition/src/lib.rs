//! Assembly root: stitches the domain crates into the plugin container.
//!
//! Reference: `dsh packages/bundle/base` — a bundle module whose one job is
//! registering the core services and starting plugins, in order. The daemon
//! Worker + SessionDb wiring still lives in the daemon crate; this root
//! covers the harness family plus the agent-domain thin layers.

use std::sync::Arc;

use serde_json::Value;

use omenic_harness_plugin::{DshPlugin, Fiber, PluginError, PluginRegistry};
use omenic_harness_prompt::PromptTemplate;
use omenic_harness_runtime::LoopEngine;

/// Build the plugin container: a [`Fiber`] over `config`, every harness
/// crate's services registered in dependency order, then the orbit thin
/// layer, then the host `plugins` (a duplicate name aborts with
/// [`PluginError`] and the caller discards the partially built fiber).
pub fn assemble(
    config: Value,
    plugins: Vec<Arc<dyn DshPlugin>>,
) -> Result<(Fiber, PluginRegistry), PluginError> {
    let mut fiber = Fiber::with_config(config);
    let mut registry = PluginRegistry::default();
    {
        let ctx = &mut fiber.context();
        // `harness-core` is vocabulary only — types, no service instances.
        // 1. prompt: the base system template.
        let system = ctx.config()["system_prompt"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        ctx.provide("harness.prompt", PromptTemplate::new("base", system));
        // 2. tools: the built-in tool catalog.
        ctx.provide("harness.tools", omenic_harness_tools::default_catalog());
        // 3. runtime: the loop engine, tuned from config.
        let model = ctx.config()["model"]
            .as_str()
            .unwrap_or("default")
            .to_string();
        let max_turns = ctx.config()["max_turns"].as_u64().unwrap_or(64) as usize;
        ctx.provide("harness.loop", LoopEngine { max_turns, model });
        // 4. agent domain (allowed direction: agent → harness): orbit defaults.
        orbit::register(ctx);
        // 5. host plugins last: they may provide over anything above.
        for plugin in plugins {
            registry.register(plugin, ctx)?;
        }
    }
    Ok((fiber, registry))
}
