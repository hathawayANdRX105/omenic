//! Skill plugin: discovers skills at register time, provides SKILL_SERVICE,
//! registers SkillTool via harness.tools.

use std::path::PathBuf;
use std::sync::Arc;

use omenic_harness_plugin::{DshPlugin, PluginContext};
use omenic_harness_tools::ToolCatalog;

use crate::catalog::{SKILL_SERVICE, SkillService};
use crate::tool::SkillTool;

/// Skill plugin providing the skill service and tool.
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
