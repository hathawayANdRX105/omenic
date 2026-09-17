use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::provider::SubagentProvider;

/// Thread-safe registry of named subagent providers.
///
/// Stored in the fiber under the key `"harness.subagents"`. Callers resolve
/// the service and query it by provider name.
#[derive(Default)]
pub struct SubagentRuntimeService {
    providers: Mutex<HashMap<String, Arc<dyn SubagentProvider>>>,
}

impl SubagentRuntimeService {
    /// Register `provider` under `name`. Overwrites an existing entry with the
    /// same name (composition controls load order).
    pub fn register(&self, name: impl Into<String>, provider: Arc<dyn SubagentProvider>) {
        self.providers.lock().unwrap().insert(name.into(), provider);
    }

    /// Look up a provider by name.
    pub fn get(&self, name: &str) -> Option<Arc<dyn SubagentProvider>> {
        self.providers.lock().unwrap().get(name).cloned()
    }

    /// All registered provider names, in insertion order.
    pub fn providers(&self) -> Vec<String> {
        self.providers.lock().unwrap().keys().cloned().collect()
    }
}

/// Plugin that wires `SubagentRuntimeService` into the fiber.
///
/// Register this plugin in the composition root (Task C) so downstream
/// plugins and tools can resolve `"harness.subagents"` and register or
/// query subagent providers.
#[derive(Default)]
pub struct SubagentRuntime;

impl omenic_harness_plugin::DshPlugin for SubagentRuntime {
    fn name(&self) -> &str {
        "harness.subagents"
    }

    fn register(&self, ctx: &mut omenic_harness_plugin::PluginContext<'_>) {
        ctx.provide("harness.subagents", SubagentRuntimeService::default());
    }
}
