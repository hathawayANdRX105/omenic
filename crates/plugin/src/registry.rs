//! Plugin trait + named plugin registry.
//!
//! Reference: `dsh vendor/cordis/src/registry.ts` (`RegistryService.start`):
//! a plugin is a named unit; `apply(ctx)` wires it into the context, and a
//! duplicate name is rejected before anything runs.

use crate::context::PluginContext;
use serde_json::Value;

/// A named unit of harness functionality.
///
/// Reference: cordis `Plugin.Function` — `(ctx) => void` entrypoint plus a
/// display name. Registration is synchronous; long-running work belongs in
/// the services the plugin provides, not in `register`.
pub trait DshPlugin: Send + Sync {
    /// Unique plugin name (duplicate names are rejected).
    fn name(&self) -> &str;
    /// Wire this plugin's services/subscriptions into the context.
    fn register(&self, ctx: &mut PluginContext<'_>);
    /// Validate `config` against this plugin's schema before registration.
    ///
    /// The default implementation accepts any config. Override to enforce
    /// per-plugin schemas; return `Err(PluginError::InvalidConfig(…))` to
    /// reject before `register` is called.
    fn validate_config(&self, _config: &Value) -> Result<(), PluginError> {
        Ok(())
    }
}

/// Registration failure.
#[derive(Debug)]
pub enum PluginError {
    /// A plugin with the same name is already registered.
    Duplicate(String),
    /// Schema validation failed before registration.
    InvalidConfig(String),
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginError::Duplicate(name) => write!(f, "duplicate plugin name: {name}"),
            PluginError::InvalidConfig(msg) => write!(f, "plugin config invalid: {msg}"),
        }
    }
}

impl std::error::Error for PluginError {}

/// Ordered list of installed plugins, rejecting name collisions.
#[derive(Default)]
pub struct PluginRegistry {
    plugins: Vec<std::sync::Arc<dyn DshPlugin>>,
}

impl PluginRegistry {
    /// Register `plugin`: the duplicate check runs **before**
    /// `plugin.register`, so a rejected plugin never mutates the context.
    pub fn register(
        &mut self,
        plugin: std::sync::Arc<dyn DshPlugin>,
        ctx: &mut PluginContext<'_>,
    ) -> Result<(), PluginError> {
        self.do_register(plugin, ctx, &Value::Null)
    }

    /// Register `plugin` with an explicit config document. The duplicate check
    /// and schema validation both run before `plugin.register`, so a rejected
    /// plugin never mutates the context.
    pub fn register_with_config(
        &mut self,
        plugin: std::sync::Arc<dyn DshPlugin>,
        ctx: &mut PluginContext<'_>,
        config: &Value,
    ) -> Result<(), PluginError> {
        self.do_register(plugin, ctx, config)
    }

    fn do_register(
        &mut self,
        plugin: std::sync::Arc<dyn DshPlugin>,
        ctx: &mut PluginContext<'_>,
        config: &Value,
    ) -> Result<(), PluginError> {
        if self.plugins.iter().any(|p| p.name() == plugin.name()) {
            return Err(PluginError::Duplicate(plugin.name().to_string()));
        }
        if let Err(e) = plugin.validate_config(config) {
            return Err(e);
        }
        plugin.register(ctx);
        self.plugins.push(plugin);
        Ok(())
    }

    /// Unregister `plugin` by name: removes it from the registry and returns
    /// the plugin definition. The caller is responsible for any cleanup the
    /// plugin wired into the fiber (services, subscriptions). Returns `Ok(None)`
    /// when no plugin with that name is registered (idempotent).
    pub fn unregister(
        &mut self,
        name: &str,
    ) -> Result<Option<std::sync::Arc<dyn DshPlugin>>, PluginError> {
        if let Some(pos) = self.plugins.iter().position(|p| p.name() == name) {
            Ok(Some(self.plugins.remove(pos)))
        } else {
            Ok(None)
        }
    }

    /// Registered plugin names, in registration order.
    pub fn plugins(&self) -> Vec<&str> {
        self.plugins.iter().map(|p| p.name()).collect()
    }

    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }
}
