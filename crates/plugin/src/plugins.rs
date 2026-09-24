//! Consolidated DshPlugin implementations for the plugin crate.
//!
//! Order: compaction → instruction → guard → skill → plan-mode (as specified).
//!
//! Each plugin provides a single service under its name domain in the harness.
//! Host plugins run after these core plugins, so they can override their services.

use crate::context::PluginContext;
use crate::{DshPlugin, PluginError};
use serde_json::Value;

use compaction::CharBudgetPolicy;
use guard::{GUARD_SERVICE, GuardConfig, GuardService};
use instruction::{INSTRUCTION_SERVICE, InstructionCache, InstructionFragments, render_fragments};
use plan_mode::PlanModeConfig;
use plan_mode::core::{PLAN_MODE_SERVICE, PlanModeService};
use plan_mode::state::PlanModeRuntime;
use skill::catalog::{SKILL_SERVICE, SkillService};
use skill::tool::SkillTool;
use std::path::PathBuf;
use std::sync::Arc;

use tools_harness::ToolCatalog;

/// Compaction plugin: provides the default char-budget policy as the
/// `harness.compaction` service.
pub struct CompactionPlugin;

impl DshPlugin for CompactionPlugin {
    fn name(&self) -> &str {
        "harness-compaction"
    }

    fn register(&self, ctx: &mut PluginContext<'_>) {
        ctx.provide("harness.compaction", CharBudgetPolicy::default());
    }
}

/// Instruction plugin: provides rendered workspace fragments as the
/// `harness.instruction` service.
pub struct InstructionPlugin {
    cwd: PathBuf,
}

impl Default for InstructionPlugin {
    fn default() -> Self {
        Self::new(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }
}

impl InstructionPlugin {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        InstructionPlugin { cwd: cwd.into() }
    }

    /// Load (mtime-cached) + render (digest-deduped) right now.
    pub fn fragments(&self, cache: &mut InstructionCache) -> InstructionFragments {
        InstructionFragments(render_fragments(&cache.load(&self.cwd)))
    }
}

impl DshPlugin for InstructionPlugin {
    fn name(&self) -> &str {
        "harness-instruction"
    }

    fn register(&self, ctx: &mut PluginContext<'_>) {
        ctx.provide(
            INSTRUCTION_SERVICE,
            self.fragments(&mut InstructionCache::default()),
        );
    }
}

/// Guard plugin that registers the guard service and any associated tools.
pub struct GuardPlugin {
    config: GuardConfig,
}

impl GuardPlugin {
    pub fn new(config: GuardConfig) -> Result<Self, Box<dyn std::error::Error>> {
        // Validate repeat config (timeout validation happens in TimeoutPolicy::new)
        let _reminder = guard::RepeatToolReminder::new(config.repeat.clone())?;
        let _timeout = guard::TimeoutPolicy::new(config.timeout.clone())?;
        Ok(Self { config })
    }
}

impl DshPlugin for GuardPlugin {
    fn name(&self) -> &str {
        "guard"
    }

    fn register(&self, ctx: &mut PluginContext<'_>) {
        let reminder = guard::RepeatToolReminder::new(self.config.repeat.clone())
            .expect("repeat config already validated");
        let timeout = guard::TimeoutPolicy::new(self.config.timeout.clone())
            .expect("timeout config already validated");

        let service = GuardService {
            reminder,
            timeout,
            snapshot: Arc::new(parking_lot::RwLock::new(vec![])),
        };

        ctx.provide(GUARD_SERVICE, service);
    }

    fn validate_config(&self, _config: &Value) -> Result<(), PluginError> {
        // Config validation happens at GuardPlugin::new time; accept any for now
        Ok(())
    }
}

/// Skill plugin: discovers skills at cwd, provides SKILL_SERVICE,
/// registers SkillTool via harness.tools.
pub struct SkillPlugin {
    cwd: PathBuf,
}

impl Default for SkillPlugin {
    fn default() -> Self {
        Self::new(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }
}

impl SkillPlugin {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self { cwd: cwd.into() }
    }
}

impl DshPlugin for SkillPlugin {
    fn name(&self) -> &str {
        "harness-skill"
    }

    fn register(&self, ctx: &mut PluginContext<'_>) {
        let service = SkillService::with_cwd(&self.cwd);
        let arc_service = Arc::new(service.clone());
        ctx.provide(SKILL_SERVICE, service);

        // Register the skill tool via harness.tools (matches instruction plugin pattern)
        if let Some(catalog) = ctx.resolve::<ToolCatalog>("harness.tools") {
            let tool = SkillTool::new(arc_service);
            catalog.register(Arc::new(tool));
        }
    }
}

/// Plan mode plugin: hosts the plan-mode service and the exit_plan_mode tool.
pub struct PlanModePlugin {
    runtime: PlanModeRuntime,
    config: PlanModeConfig,
}

impl PlanModePlugin {
    /// Create a new plan mode plugin.
    #[must_use]
    pub fn new(config: PlanModeConfig) -> Self {
        let active = false; // default inactive
        let runtime = PlanModeRuntime::new(active);
        Self { runtime, config }
    }

    /// Create a new plan mode plugin with initial active state.
    #[must_use]
    pub fn new_with_active(config: PlanModeConfig, active: bool) -> Self {
        let runtime = PlanModeRuntime::new(active);
        Self { runtime, config }
    }

    /// Shared handle to the plugin's plan-mode state.
    #[must_use]
    pub fn runtime(&self) -> &PlanModeRuntime {
        &self.runtime
    }
}

impl DshPlugin for PlanModePlugin {
    fn name(&self) -> &str {
        "harness-plan-mode"
    }

    fn validate_config(&self, config: &Value) -> Result<(), PluginError> {
        // Config slice at config["plan"]; absent slice keeps the
        // constructor config (the daemon always supplies a section), a
        // present but invalid slice rejects — same shape as the guard
        // plugin's config["guard"] handling.
        match config.get("plan") {
            None => Ok(()),
            Some(value) if value.is_null() => Ok(()),
            Some(value) => {
                let section = value.get("section").and_then(Value::as_str);
                match section {
                    Some(section) if !section.trim().is_empty() => Ok(()),
                    _ => Err(PluginError::InvalidConfig(
                        "plan.section must be a non-empty string".to_string(),
                    )),
                }
            }
        }
    }

    fn register(&self, ctx: &mut PluginContext<'_>) {
        // Resolve review port from config or use default
        let review_port = self
            .config
            .review_port
            .clone()
            .unwrap_or_else(|| Arc::new(plan_mode::AutoDenyReview));

        // Section: doc slice config["plan"]["section"] wins (same shape as
        // the guard plugin's config["guard"]); the constructor config is
        // the fallback — the daemon always supplies one.
        let section = ctx
            .config()
            .get("plan")
            .and_then(|plan| plan.get("section"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| self.config.section.clone())
            .unwrap_or_else(|| {
                "You are in plan mode. Explore and design before presenting the complete plan through exit_plan_mode.".to_string()
            });

        // Create service
        let service =
            PlanModeService::new(self.runtime.clone(), review_port.clone(), section.clone());

        // Provide service
        ctx.provide(PLAN_MODE_SERVICE, service);

        // Register exit_plan_mode tool
        let catalog = ctx
            .resolve::<ToolCatalog>("harness.tools")
            .expect("harness.tools must be provided by composition");
        catalog.register(Arc::new(plan_mode::tools::exit_plan_mode_tool(
            self.runtime.clone(),
            review_port,
        )));
    }
}
