//! Repeat Tool Reminder: advisory loop-breaker, pure logic, no I/O

use parking_lot::Mutex;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;

use crate::glob::glob_match;

/// Configuration for RepeatToolReminder
#[derive(Debug, Clone)]
pub struct RepeatConfig {
    /// Consecutive call counts that trigger reminders (ascending, >=2, no duplicates)
    pub thresholds: Vec<usize>,
    /// Tool name patterns to track; empty => all tools
    pub include: Vec<String>,
    /// Tool name patterns transparent to chain; default ["todo_write"]
    pub exclude: Vec<String>,
    /// Cap on argument preview in detailed reminders (>=1)
    pub arguments_preview_chars: usize,
}

impl Default for RepeatConfig {
    fn default() -> Self {
        Self {
            thresholds: vec![3, 5, 8],
            include: vec![],
            exclude: vec!["todo_write".to_string()],
            arguments_preview_chars: 500,
        }
    }
}

/// Errors from RepeatConfig validation
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("thresholds must be a non-empty list")]
    EmptyThresholds,
    #[error("thresholds must be integers >= 2")]
    ThresholdBelowTwo,
    #[error("thresholds contains duplicate value: {0}")]
    DuplicateThreshold(usize),
    #[error("thresholds must be in ascending order (after normalization)")]
    NotAscending,
    #[error("arguments_preview_chars must be >= 1")]
    InvalidPreviewChars,
}

/// Canonicalizes JSON arguments: deep key-sort + compact serialization
fn canonical_args(args: &Value) -> String {
    fn sort_value(v: &Value) -> Value {
        match v {
            Value::Object(map) => {
                let mut sorted: Map<String, Value> = Map::new();
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                for k in keys {
                    sorted.insert(k.clone(), sort_value(&map[k]));
                }
                Value::Object(sorted)
            }
            Value::Array(arr) => Value::Array(arr.iter().map(sort_value).collect()),
            _ => v.clone(),
        }
    }
    let sorted = sort_value(args);
    serde_json::to_string(&sorted).unwrap_or_default()
}

/// Checks if a tool name matches include/exclude patterns
fn is_tracked(tool: &str, include: &[String], exclude: &[String]) -> bool {
    // Exclude takes precedence
    for pat in exclude {
        if glob_match(pat, tool) {
            return false;
        }
    }
    // If include is empty, track all (except excluded)
    if include.is_empty() {
        return true;
    }
    // Otherwise, must match at least one include pattern
    for pat in include {
        if glob_match(pat, tool) {
            return true;
        }
    }
    false
}

/// Reminder message templates matching dsh spec exactly
const FIRST_THRESHOLD_MSG: &str = "You are repeating the exact same tool call with identical arguments. Carefully analyze the previous result before calling again: if the task is not complete, try a different approach or different arguments instead of repeating the call.";

const LATER_THRESHOLD_TEMPLATE: &str = "Repeated tool call detected:
- tool: {tool}
- consecutive_calls: {count}
- arguments: {args}
The repeated calls are not making progress. Do not call this tool with these exact arguments again. Inspect the latest result and choose a different action, different arguments, or finish the task if enough evidence has been gathered.";

/// Chain state per agent
struct ChainState {
    tool: String,
    canonical_args: String,
    count: usize,
}

/// State shared across agents
struct SharedState {
    config: RepeatConfig,
    /// Per-agent chain state: agent_id -> ChainState
    chains: Mutex<HashMap<String, ChainState>>,
}

/// RepeatToolReminder: advisory loop-breaker, pure logic, no I/O
#[derive(Clone)]
pub struct RepeatToolReminder {
    shared: Arc<SharedState>,
}

impl RepeatToolReminder {
    /// Creates a new RepeatToolReminder with validated config.
    /// Fails loud on invalid config (empty thresholds, <2, duplicates, non-ascending, invalid preview chars).
    pub fn new(config: RepeatConfig) -> Result<Self, ConfigError> {
        // Validate thresholds
        if config.thresholds.is_empty() {
            return Err(ConfigError::EmptyThresholds);
        }
        for &t in &config.thresholds {
            if t < 2 {
                return Err(ConfigError::ThresholdBelowTwo);
            }
        }
        // Check duplicates
        let mut seen = std::collections::HashSet::new();
        for &t in &config.thresholds {
            if !seen.insert(t) {
                return Err(ConfigError::DuplicateThreshold(t));
            }
        }
        // Normalize to ascending and check
        let mut sorted = config.thresholds.clone();
        sorted.sort();
        if sorted != config.thresholds {
            return Err(ConfigError::NotAscending);
        }
        // Validate preview chars
        if config.arguments_preview_chars < 1 {
            return Err(ConfigError::InvalidPreviewChars);
        }

        Ok(Self {
            shared: Arc::new(SharedState {
                config,
                chains: Mutex::new(HashMap::new()),
            }),
        })
    }

    /// Observes a tool call, returns reminder message if threshold crossed.
    /// Chain key = (tool name, canonical args). Excluded calls are transparent (no increment, no reset).
    pub fn observe(&self, agent: &str, tool: &str, args: &Value) -> Option<String> {
        let config = &self.shared.config;

        // Check if tool is tracked
        if !is_tracked(tool, &config.include, &config.exclude) {
            return None;
        }

        let canonical = canonical_args(args);
        let mut chains = self.shared.chains.lock();
        let chain = chains
            .entry(agent.to_string())
            .or_insert_with(|| ChainState {
                tool: tool.to_string(),
                canonical_args: canonical.clone(),
                count: 0,
            });

        // Same tool + same canonical args => increment
        if chain.tool == tool && chain.canonical_args == canonical {
            chain.count += 1;
        } else {
            // Different tracked call => reset to 1
            chain.tool = tool.to_string();
            chain.canonical_args = canonical.clone();
            chain.count = 1;
        }

        let count = chain.count;

        // Find which threshold (if any) this count matches
        let threshold_idx = config.thresholds.iter().position(|&t| t == count);

        threshold_idx.map(|idx| {
            if idx == 0 {
                FIRST_THRESHOLD_MSG.to_string()
            } else {
                let args_preview = if canonical.len() > config.arguments_preview_chars {
                    // ponytail: byte-budget cut, advanced to the next char boundary
                    // so multi-byte tool args cannot panic the slice.
                    let mut end = config.arguments_preview_chars;
                    while end < canonical.len() && !canonical.is_char_boundary(end) {
                        end += 1;
                    }
                    let omitted = canonical.len() - end;
                    format!("{}… (+{} more chars)", &canonical[..end], omitted)
                } else {
                    canonical.clone()
                };
                LATER_THRESHOLD_TEMPLATE
                    .replace("{tool}", tool)
                    .replace("{count}", &count.to_string())
                    .replace("{args}", &args_preview)
            }
        })
    }

    /// Returns a snapshot of all agent chain states (for testing/daemon queries)
    pub fn snapshot(&self) -> Vec<AgentChainSnapshot> {
        let chains = self.shared.chains.lock();
        chains
            .iter()
            .map(|(agent, chain)| AgentChainSnapshot {
                agent: agent.clone(),
                tool: chain.tool.clone(),
                canonical_args: chain.canonical_args.clone(),
                count: chain.count,
            })
            .collect()
    }

    /// Resets chain for a specific agent (e.g., on user prompt)
    pub fn reset_agent(&self, agent: &str) {
        self.shared.chains.lock().remove(agent);
    }
}

/// Snapshot of an agent's chain state
#[derive(Debug, Clone)]
pub struct AgentChainSnapshot {
    pub agent: String,
    pub tool: String,
    pub canonical_args: String,
    pub count: usize,
}
