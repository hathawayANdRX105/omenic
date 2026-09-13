//! Plugin context + service container.
//!
//! Reference: `dsh vendor/cordis/src/context.ts` (`Context.provide/resolve`
//! via the reflect layer) and `service.ts` (`Service` registers itself by
//! name into the owning context).
//!
//! Rust simplification: services are keyed by `TypeId`, not by name — one
//! instance per concrete type. The `key` argument records the registration
//! name and must match on lookup, so a wrong-type take (`resolve::<Wrong>`)
//! returns `None` instead of panicking.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use crate::events::{EventBus, EventHandler, Subscription};

/// Type-keyed static service registry (cordis `ReflectService.provide`).
///
/// Constraint: one service per concrete type; re-providing the same type
/// replaces the entry. Wrong-type or wrong-key lookups return `None`.
#[derive(Default)]
pub struct ServiceRegistry {
    services: HashMap<TypeId, (String, Arc<dyn Any + Send + Sync>)>,
}

impl ServiceRegistry {
    /// Register `value` under its concrete type, labelled `key`.
    pub fn provide<T: Any + Send + Sync>(&mut self, key: &str, value: T) {
        self.services
            .insert(TypeId::of::<T>(), (key.to_string(), Arc::new(value)));
    }

    /// Look up the `T` service registered under `key`.
    ///
    /// Returns `None` when no `T` was provided or when it was provided under
    /// a different key (type mismatch = the cordis "service unavailable" case,
    /// a wrong-type take, never a panic).
    pub fn resolve<T: Any + Send + Sync>(&self, key: &str) -> Option<Arc<T>> {
        let (name, erased) = self.services.get(&TypeId::of::<T>())?;
        if name != key {
            return None;
        }
        Arc::clone(erased).downcast::<T>().ok()
    }
}

/// Borrowed assembly view handed to plugins: registry + event bus + a
/// read-only config document.
///
/// Reference: `cordis Context` — plugins only ever see the context, never
/// the owning fiber.
pub struct PluginContext<'a> {
    registry: &'a mut ServiceRegistry,
    bus: &'a mut EventBus,
    config: &'a Value,
}

impl<'a> PluginContext<'a> {
    pub fn new(
        registry: &'a mut ServiceRegistry,
        bus: &'a mut EventBus,
        config: &'a Value,
    ) -> Self {
        Self {
            registry,
            bus,
            config,
        }
    }

    pub fn provide<T: Any + Send + Sync>(&mut self, key: &str, value: T) {
        self.registry.provide(key, value);
    }

    pub fn resolve<T: Any + Send + Sync>(&self, key: &str) -> Option<Arc<T>> {
        self.registry.resolve(key)
    }

    /// Publish an event on a topic: synchronous, subscription order.
    pub fn emit(&self, topic: &str, event: &dyn Any) {
        self.bus.emit(topic, event);
    }

    /// Subscribe `handler` to `topic`; keep the token to unsubscribe later.
    pub fn subscribe(&mut self, topic: impl Into<String>, handler: EventHandler) -> Subscription {
        self.bus.subscribe(topic, handler)
    }

    /// Remove a previously subscribed handler (plugin unload path).
    pub fn unsubscribe(&mut self, sub: Subscription) {
        self.bus.unsubscribe(sub);
    }

    /// Read-only view of the merged config document.
    pub fn config(&self) -> &Value {
        self.config
    }
}
