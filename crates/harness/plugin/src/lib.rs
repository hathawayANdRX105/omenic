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

// Re-exports: plugins speak the harness vocabulary without declaring the
// core/runtime crates themselves.
pub use omenic_harness_core as core;
pub use omenic_harness_runtime as runtime;
