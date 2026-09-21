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
        // Most specific match wins — deterministic regardless of the
        // HashMap's iteration order: fewer wildcards first, then more
        // literal characters ("*.py" beats "*" for "script.py"; an exact
        // name beats every pattern).
        let mut best: Option<((usize, std::cmp::Reverse<usize>), u64)> = None;
        for (pattern, &seconds) in &self.config.rules {
            if !glob_match(pattern, tool) {
                continue;
            }
            let wildcards = pattern.matches('*').count();
            let literal = pattern.len() - wildcards;
            let key = (wildcards, std::cmp::Reverse(literal));
            if best.is_none_or(|(best_key, _)| key < best_key) {
                best = Some((key, seconds));
            }
        }
        best.map(|(_, seconds)| Duration::from_secs(seconds))
    }

    /// Returns all configured rules (for testing/daemon queries)
    pub fn rules(&self) -> &HashMap<String, u64> {
        &self.config.rules
    }
}
