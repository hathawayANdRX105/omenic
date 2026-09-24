//! Loop-hygiene guards: the repeat-tool-reminder advisory nudge and the
//! timeout-policy per-call deadline listener.
//!
//! Reference: dsh `packages/guard/repeat-tool-reminder` and
//! `packages/guard/timeout-policy` (behavioral specs).

mod repeat_tool_reminder;
mod timeout_policy;

pub mod glob;

pub use glob::glob_match;
pub use repeat_tool_reminder::{
    AgentChainSnapshot, ConfigError as RepeatConfigError, RepeatConfig, RepeatToolReminder,
};
pub use timeout_policy::{ConfigError as TimeoutConfigError, TimeoutConfig, TimeoutPolicy};

use parking_lot::RwLock;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

/// Service key for the guard service (matches contract).
pub const GUARD_SERVICE: &str = "guard.service";

/// Guard service exposing reminder state snapshot and timeout queries for
/// daemon/testing use.
#[derive(Clone)]
pub struct GuardService {
    pub reminder: RepeatToolReminder,
    pub timeout: TimeoutPolicy,
    pub snapshot: Arc<RwLock<Vec<AgentChainSnapshot>>>,
}

impl GuardService {
    pub fn reminder(&self) -> &RepeatToolReminder {
        &self.reminder
    }

    pub fn timeout_policy(&self) -> &TimeoutPolicy {
        &self.timeout
    }

    /// Returns current snapshot of agent chains (for daemon queries/tests).
    pub fn snapshot(&self) -> Vec<AgentChainSnapshot> {
        self.snapshot.read().clone()
    }

    /// Convenience for checking repeat reminder.
    pub fn check_repeat(&self, agent: &str, tool: &str, args: &Value) -> Option<String> {
        self.reminder.observe(agent, tool, args)
    }

    /// Convenience for deadline lookup.
    pub fn deadline_for(&self, tool: &str) -> Option<std::time::Duration> {
        self.timeout.deadline_for(tool)
    }
}

/// Guard config with repeat and timeout settings.
#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GuardConfig {
    pub repeat: RepeatConfig,
    pub timeout: TimeoutConfig,
}

impl Default for GuardConfig {
    fn default() -> Self {
        Self {
            repeat: RepeatConfig::default(),
            timeout: TimeoutConfig::default(),
        }
    }
}
