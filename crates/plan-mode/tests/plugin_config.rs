//! Plugin registration: service, tool, and config validation.

use std::sync::Arc;

use plan_mode::{PLAN_MODE_SERVICE, PlanModeService};
use plugin::plugins::PlanModePlugin;
use plugin::{DshPlugin, PluginContext, ServiceRegistry};
use serde_json::json;
use tools_harness::ToolCatalog;

fn plugin() -> PlanModePlugin {
    PlanModePlugin::new(plan_mode::PlanModeConfig {
        section: Some("plan guidance".to_string()),
        review_port: None,
    })
}

#[test]
fn plugin_name_is_stable() {
    assert_eq!(plugin().name(), "harness-plan-mode");
}

#[test]
fn config_validation_checks_the_plan_slice() {
    let p = plugin();
    // Named-slice convention (same as guard's config["guard"]): an absent
    // slice keeps the constructor config (the daemon registers through
    // `registry.register`, which passes Value::Null), a present slice must
    // carry a non-empty string section.
    assert!(p.validate_config(&json!({})).is_ok());
    assert!(p.validate_config(&serde_json::Value::Null).is_ok());
    assert!(p.validate_config(&json!({ "plan": {} })).is_err());
    assert!(
        p.validate_config(&json!({ "plan": { "section": "" } }))
            .is_err()
    );
    assert!(
        p.validate_config(&json!({ "plan": { "section": "   " } }))
            .is_err()
    );
    assert!(
        p.validate_config(&json!({ "plan": { "section": 42 } }))
            .is_err()
    );
    assert!(
        p.validate_config(&json!({ "plan": { "section": "plan guidance" } }))
            .is_ok()
    );
}

#[test]
fn register_provides_service_and_tool() {
    let mut registry = ServiceRegistry::default();
    registry.provide("harness.tools", ToolCatalog::default());

    let p = plugin();
    let config = json!({});
    let mut bus = plugin::EventBus::new();
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
        plan_mode::PlanModeConfig {
            section: Some("plan guidance".to_string()),
            review_port: None,
        },
        true,
    );
    let mut registry = ServiceRegistry::default();
    registry.provide("harness.tools", ToolCatalog::default());
    let config = json!({});
    let mut bus = plugin::EventBus::new();
    let mut ctx = PluginContext::new(&mut registry, &mut bus, &config);
    p.register(&mut ctx);

    let service: Arc<PlanModeService> = ctx.resolve(PLAN_MODE_SERVICE).unwrap();
    assert_eq!(service.plan_policy_section(), "plan guidance");
}
