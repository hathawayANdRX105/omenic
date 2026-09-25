//! Plugin surface for the omenic agent harness (C6).
//!
//! Reference: `dsh vendor/cordis` — the dependency container the dsh core
//! runs on: [`ServiceRegistry`] (typed `provide`/`resolve`), [`EventBus`]
//! (synchronous topic bus), [`Fiber`] + [`PluginLifecycle`] (LIFO teardown),
//! and [`DshPlugin`] + [`PluginRegistry`] (named registration, duplicate
//! rejection). The composition root (`crates/composition`) assembles these;
//! domain crates expose a `register` thin layer on top.
//!
//! Constraint: harness-domain crate — depends only on other `crates/harness`
//! members, never on the agent domain.

mod context;
mod events;
mod fiber;
mod registry;

pub use context::{PluginContext, ServiceRegistry};
pub use events::{EventBus, EventHandler, Subscription};
pub use fiber::{Fiber, PluginLifecycle};
pub use registry::{DshPlugin, PluginError, PluginRegistry};
pub mod composition;
pub mod plugins;

/// Composition root (old `composition` crate): assemble the plugin container.
pub use composition::assemble;
