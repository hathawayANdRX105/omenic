//! Plan mode plugin for the harness.

use std::sync::Arc;

use plugin::{DshPlugin, PluginContext};
use serde_json::Value;
use tools_harness::ToolCatalog;

use crate::core::{PLAN_MODE_SERVICE, PlanModeService};
use crate::port::{AutoDenyReview, DynPlanReviewPort};
use crate::state::PlanModeRuntime;
use crate::tools::exit_plan_mode_tool;

/// Configuration for the plan mode plugin.
#[derive(Clone, Default)]
pub struct PlanModeConfig {
    /// The plan policy section text (required, non-empty).
    pub section: Option<String>,
    /// Optional custom review port (if not provided, AutoDenyReview is used).
    pub review_port: Option<DynPlanReviewPort>,
}

impl PlanModeConfig {
    /// Validate the configuration.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(ref section) = self.section {
            if section.trim().is_empty() {
                return Err("section must be non-empty".to_string());
            }
        } else {
            return Err("section is required".to_string());
        }
        Ok(())
    }
}

/// Plan mode plugin.
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

    /// Shared handle to the plugin's plan-mode state. The host (daemon) keeps
    /// one to parse `/plan` commands between turns without re-registering.
    #[must_use]
    pub fn runtime(&self) -> &PlanModeRuntime {
        &self.runtime
    }
}

impl DshPlugin for PlanModePlugin {
    fn name(&self) -> &str {
        "harness-plan-mode"
    }

    fn validate_config(&self, config: &Value) -> Result<(), plugin::PluginError> {
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
                    _ => Err(plugin::PluginError::InvalidConfig(
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
            .unwrap_or_else(|| Arc::new(AutoDenyReview));

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
        catalog.register(Arc::new(exit_plan_mode_tool(
            self.runtime.clone(),
            review_port,
        )));
    }
}
