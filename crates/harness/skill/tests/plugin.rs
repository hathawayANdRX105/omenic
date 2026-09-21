//! SkillPlugin registration: service + tool land in the assembled container.

use std::sync::Arc;

use omenic_harness_plugin::{DshPlugin, EventBus, PluginContext, ServiceRegistry};
use omenic_harness_skill::{SKILL_SERVICE, SkillPlugin, SkillService};
use omenic_harness_tools::ToolCatalog;
use serde_json::json;
use tempfile::TempDir;

#[test]
fn plugin_name_is_stable() {
    let tmp = TempDir::new().unwrap();
    assert_eq!(SkillPlugin::new(tmp.path()).name(), "harness-skill");
}

#[test]
fn register_provides_service_and_tool() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join(".dsh/skills")).unwrap();
    std::fs::write(
        tmp.path().join(".dsh/skills/demo.md"),
        "---\nname: demo\ndescription: Demo skill\n---\nBody.",
    )
    .unwrap();

    let mut registry = ServiceRegistry::default();
    registry.provide("harness.tools", ToolCatalog::default());

    let plugin = SkillPlugin::new(tmp.path());
    let config = json!({});
    let mut bus = EventBus::new();
    let mut ctx = PluginContext::new(&mut registry, &mut bus, &config);
    plugin.register(&mut ctx);

    let service: Option<Arc<SkillService>> = ctx.resolve(SKILL_SERVICE);
    let service = service.expect("skill service provided");
    let entries = service.entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "demo");

    let catalog = ctx
        .resolve::<ToolCatalog>("harness.tools")
        .expect("harness.tools provided");
    assert!(catalog.find("skill").is_some(), "skill tool registered");
}

#[test]
fn register_tolerates_missing_tools_service() {
    let tmp = TempDir::new().unwrap();
    let mut registry = ServiceRegistry::default();
    let plugin = SkillPlugin::new(tmp.path());
    let config = json!({});
    let mut bus = EventBus::new();
    let mut ctx = PluginContext::new(&mut registry, &mut bus, &config);

    // No harness.tools provided: registration must still land the service.
    plugin.register(&mut ctx);
    let service: Option<Arc<SkillService>> = ctx.resolve(SKILL_SERVICE);
    assert!(service.is_some());
}
