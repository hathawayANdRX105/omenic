//! Plugin trait + named plugin registry.
//!
//! Reference: `dsh vendor/cordis/src/registry.ts` (`RegistryService.start`):
//! a plugin is a named unit; `apply(ctx)` wires it into the context, and a
//! duplicate name is rejected before anything runs.

use crate::context::PluginContext;

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
}

/// Registration failure.
#[derive(Debug)]
pub enum PluginError {
    /// A plugin with the same name is already registered.
    Duplicate(String),
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginError::Duplicate(name) => write!(f, "duplicate plugin name: {name}"),
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
        if self.plugins.iter().any(|p| p.name() == plugin.name()) {
            return Err(PluginError::Duplicate(plugin.name().to_string()));
        }
        plugin.register(ctx);
        self.plugins.push(plugin);
        Ok(())
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
