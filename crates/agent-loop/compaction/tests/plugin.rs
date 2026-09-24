//! Plugin face: the policy resolves as the `harness.compaction` service.

use serde_json::Value;

use compaction::{CharBudgetPolicy, CompactionPlugin};
use plugin::{DshPlugin, EventBus, PluginContext, ServiceRegistry};

#[test]
fn compaction_service_provides_under_its_key() {
    let mut reg = ServiceRegistry::default();
    let mut bus = EventBus::default();
    CompactionPlugin.register(&mut PluginContext::new(&mut reg, &mut bus, &Value::Null));

    assert!(
        reg.resolve::<CharBudgetPolicy>("harness.compaction")
            .is_some()
    );
    assert!(
        reg.resolve::<CharBudgetPolicy>("harness.prompt").is_none(),
        "wrong key must miss, not steal the policy"
    );
}
