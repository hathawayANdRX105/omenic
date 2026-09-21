use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

use serde::Deserialize;

use crate::glob::glob_match;

/// Configuration for TimeoutPolicy
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TimeoutConfig {
    /// Tool name glob -> timeout in seconds
    pub rules: HashMap<String, u64>,
}

/// Errors from TimeoutConfig validation
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("timeout for pattern '{pattern}' must be > 0 seconds, got {value}")]
    InvalidDuration { pattern: String, value: u64 },
}

/// TimeoutPolicy: per-call deadline mapping via tool name glob
#[derive(Clone)]
pub struct TimeoutPolicy {
    config: Arc<TimeoutConfig>,
}

impl TimeoutPolicy {
    /// Creates a new TimeoutPolicy with validated config.
    /// Fails loud on invalid config (duration <= 0).
    pub fn new(config: TimeoutConfig) -> Result<Self, ConfigError> {
        for (pattern, &value) in &config.rules {
            if value == 0 {
                return Err(ConfigError::InvalidDuration {
                    pattern: pattern.clone(),
                    value,
                });
            }
        }
        Ok(Self {
            config: Arc::new(config),
        })
    }

    /// Returns the deadline duration for a tool name, or None if no rule matches.
    /// Matches using simple glob pattern (supports `*` wildcard).
    pub fn deadline_for(&self, tool: &str) -> Option<Duration> {
        for (pattern, &seconds) in &self.config.rules {
            if glob_match(pattern, tool) {
                return Some(Duration::from_secs(seconds));
            }
        }
        None
    }

    /// Returns all configured rules (for testing/daemon queries)
    pub fn rules(&self) -> &HashMap<String, u64> {
        &self.config.rules
    }
}
