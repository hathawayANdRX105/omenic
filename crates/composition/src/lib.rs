//! Assembly root: stitches the domain crates into the plugin container.
//!
//! Reference: `dsh packages/bundle/base` — a bundle module whose one job is
//! registering the core services and starting plugins, in order. The daemon
//! Worker + SessionDb wiring still lives in the daemon crate; this root
//! covers the harness family plus the agent-domain thin layers.

use omenic_harness_compaction::CompactionPlugin;
use omenic_harness_guard::{GuardConfig, GuardPlugin};
use omenic_harness_instruction::InstructionPlugin;
use omenic_harness_prompt::PromptTemplate;
use omenic_harness_runtime::LoopEngine;
use omenic_harness_skill::SkillPlugin;
use omenic_harness_subagent::{SubagentRuntime, ToolSubagentControlPlugin, ToolSubagentPlugin};
use serde_json::Value;
use std::sync::Arc;

// Re-export the container vocabulary: a host wiring `assemble` in speaks
// only to this root, and never declares the plugin crate itself.
pub use omenic_harness_plugin::{DshPlugin, Fiber, PluginError, PluginRegistry};

/// Build the plugin container: a [`Fiber`] over `config`, every harness
/// crate's services registered in dependency order, then the orbit thin
/// layer, then the core plugins, then the host `plugins` (a duplicate name
/// aborts with [`PluginError`] and the caller discards the partially built
/// fiber).
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
        // 2b. subagent seam: runtime service, then the model-facing tools
        // (order: SubagentRuntime provides the service the tool plugins resolve).
        registry.register(Arc::new(SubagentRuntime), ctx)?;
        registry.register(Arc::new(ToolSubagentPlugin::default()), ctx)?;
        registry.register(Arc::new(ToolSubagentControlPlugin::default()), ctx)?;
        // 4. agent domain (allowed direction: agent → harness): orbit defaults.
        orbit::register(ctx);
        // 5. core plugins the framework always ships. They go through the
        // named registry (not a bare `provide`) so a host plugin reusing one
        // of their names is rejected instead of silently shadowing it.
        // Compaction lands the keep-original safe path; a host wanting a real
        // summarizer re-provides `CharBudgetPolicy` from its own plugin below.
        registry.register(Arc::new(CompactionPlugin), ctx)?;
        // Instruction discovery walks up from `cwd`; absent config means the
        // process working directory, matching `InstructionPlugin::default`.
        let instruction = match ctx.config()["cwd"].as_str() {
            Some(cwd) => InstructionPlugin::new(cwd),
            None => InstructionPlugin::default(),
        };
        registry.register(Arc::new(instruction), ctx)?;
        // Guard plugin: repeat-tool-reminder + timeout-policy. Config slice at
        // config["guard"]; absent config keeps defaults silently, a present
        // but invalid slice degrades to defaults with a warning.
        let guard_plugin = match ctx.config().get("guard") {
            Some(value) if !value.is_null() => {
                match serde_json::from_value::<GuardConfig>(value.clone()) {
                    Ok(cfg) => GuardPlugin::new(cfg).unwrap_or_else(|e| {
                        eprintln!("[guard] config invalid: {e}; using defaults");
                        GuardPlugin::new(GuardConfig::default())
                            .expect("default GuardConfig is always valid")
                    }),
                    Err(e) => {
                        eprintln!("[guard] config deserialize failed: {e}; using defaults");
                        GuardPlugin::new(GuardConfig::default())
                            .expect("default GuardConfig is always valid")
                    }
                }
            }
            _ => GuardPlugin::new(GuardConfig::default())
                .expect("default GuardConfig is always valid"),
        };
        registry.register(Arc::new(guard_plugin), ctx)?;

        // Skill plugin: discovers skills at cwd (or config["cwd"] like InstructionPlugin).
        let skill_cwd = ctx.config()["cwd"].as_str().map(std::path::PathBuf::from);
        let skill_plugin = match skill_cwd {
            Some(cwd) => SkillPlugin::new(cwd),
            None => SkillPlugin::default(),
        };
        registry.register(Arc::new(skill_plugin), ctx)?;

        // 6. host plugins last: they may provide over anything above.
        for plugin in plugins {
            registry.register(plugin, ctx)?;
        }
    }
    Ok((fiber, registry))
}
