//! Plugin registration: service, tool, and config validation.

use std::sync::Arc;

use omenic_harness_plan_mode::{PLAN_MODE_SERVICE, PlanModePlugin, PlanModeService};
use omenic_harness_plugin::{DshPlugin, PluginContext, ServiceRegistry};
use omenic_harness_tools::ToolCatalog;
use serde_json::json;

fn plugin() -> PlanModePlugin {
    PlanModePlugin::new(omenic_harness_plan_mode::PlanModeConfig {
        section: Some("plan guidance".to_string()),
        review_port: None,
    })
}

#[test]
fn plugin_name_is_stable() {
    assert_eq!(plugin().name(), "harness-plan-mode");
}

#[test]
fn config_validation_rejects_missing_or_blank_section() {
    let p = plugin();
    assert!(p.validate_config(&json!({})).is_err());
    assert!(p.validate_config(&json!({ "section": "" })).is_err());
    assert!(p.validate_config(&json!({ "section": "   " })).is_err());
    assert!(p.validate_config(&json!({ "section": 42 })).is_err());
    assert!(
        p.validate_config(&json!({ "section": "plan guidance" }))
            .is_ok()
    );
}

#[test]
fn register_provides_service_and_tool() {
    let mut registry = ServiceRegistry::default();
    registry.provide("harness.tools", ToolCatalog::default());

    let p = plugin();
    let config = json!({});
    let mut bus = omenic_harness_plugin::EventBus::new();
    let mut ctx = PluginContext::new(&mut registry, &mut bus, &config);
    p.register(&mut ctx);

    let service: Option<Arc<PlanModeService>> = ctx.resolve(PLAN_MODE_SERVICE);
    let service = service.expect("plan-mode service provided");
    assert_eq!(service.plan_policy_section(), "", "inactive by default");

    let catalog = ctx
        .resolve::<ToolCatalog>("harness.tools")
        .expect("harness.tools provided");
    let tool = catalog.find("exit_plan_mode");
    assert!(tool.is_some(), "exit_plan_mode registered");
}

#[test]
fn register_uses_configured_section_when_active() {
    let p = PlanModePlugin::new_with_active(
        omenic_harness_plan_mode::PlanModeConfig {
            section: Some("plan guidance".to_string()),
            review_port: None,
        },
        true,
    );
    let mut registry = ServiceRegistry::default();
    registry.provide("harness.tools", ToolCatalog::default());
    let config = json!({});
    let mut bus = omenic_harness_plugin::EventBus::new();
    let mut ctx = PluginContext::new(&mut registry, &mut bus, &config);
    p.register(&mut ctx);

    let service: Arc<PlanModeService> = ctx.resolve(PLAN_MODE_SERVICE).unwrap();
    assert_eq!(service.plan_policy_section(), "plan guidance");
}
