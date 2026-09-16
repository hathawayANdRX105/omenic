//! Plugin lifecycle fiber: owns the container parts and tears plugins
//! down in reverse load order.
//!
//! Reference: `dsh vendor/cordis/src/fiber.ts` (`Fiber.start` / dispose of
//! `disposables` stack LIFO, parent last).

use serde_json::Value;

use crate::context::{PluginContext, ServiceRegistry};
use crate::events::EventBus;

/// Load/unload hooks a plugin instance gets from its owning [`Fiber`].
///
/// Reference: cordis fiber lifecycle — `on_load` runs once when the fiber
/// adopts the plugin, `on_unload` runs once when it drops, in reverse
/// adoption order.
pub trait PluginLifecycle: Send {
    fn on_load(&mut self, _ctx: &mut PluginContext<'_>) {}
    fn on_unload(&mut self, _ctx: &mut PluginContext<'_>) {}
}

/// Container owning the service registry, event bus, and config document;
/// plugins loaded into it are unloaded (and their `Drop` runs) in reverse
/// registration order.
#[derive(Default)]
pub struct Fiber {
    registry: ServiceRegistry,
    bus: EventBus,
    config: Value,
    plugins: Vec<Box<dyn PluginLifecycle>>,
}

impl Fiber {
    /// Build a fiber over a pre-populated config document.
    pub fn with_config(config: Value) -> Self {
        Self {
            registry: ServiceRegistry::default(),
            bus: EventBus::new(),
            config,
            plugins: Vec::new(),
        }
    }

    /// Borrowed plugin-facing view of this fiber's parts.
    pub fn context(&mut self) -> PluginContext<'_> {
        PluginContext::new(&mut self.registry, &mut self.bus, &self.config)
    }

    /// Resolve a service out of the assembled container (read-only; the
    /// registry is untouched). This is the host-side read that makes
    /// registration real: a service a plugin `provide`d is looked up by the
    /// same key and type. `None` when the service was never provided or was
    /// provided under a different key.
    pub fn resolve<T: std::any::Any + Send + Sync>(&self, key: &str) -> Option<std::sync::Arc<T>> {
        self.registry.resolve(key)
    }

    /// Read-only view of the merged config document.
    pub fn config(&self) -> &Value {
        &self.config
    }

    /// Adopt `plugin`: runs `on_load` immediately (services first, so the
    /// plugin can resolve what earlier plugins provided).
    pub fn load(&mut self, mut plugin: Box<dyn PluginLifecycle>) {
        plugin.on_load(&mut self.context());
        self.plugins.push(plugin);
    }

    /// Live plugin count, in load order.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Tear down all adopted plugins in reverse load order. The registry,
    /// bus, and config stay alive: a host can re-load the next plugin set
    /// over the same container.
    pub fn unload(&mut self) {
        // LIFO teardown: a plugin may still depend on the ones loaded before
        // it, so the last loaded must be the first dropped.
        while let Some(mut plugin) = self.plugins.pop() {
            plugin.on_unload(&mut self.context());
        }
    }
}

impl Drop for Fiber {
    fn drop(&mut self) {
        self.unload();
    }
}
