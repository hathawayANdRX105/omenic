//! Exponential backoff — the single owner of the delay curve.
//!
//! Two callers with the same arithmetic, historically two copies: the LLM
//! call path ([`crate::openai`]) and the MCP reconnect supervisor
//! (`mcp::reconnect`). The *policies* stay separate — an LLM round-trip and
//! a transport reconnect have different retry budgets and different
//! overridability (a server's `Retry-After` only makes sense for the
//! former) — but the curve lives here so the two cannot drift.

use std::time::Duration;

/// How often and how long to retry a transport-level operation.
///
/// The LLM path layers [`crate::openai::RetryPolicy`] on top of this (it
/// also carries a per-socket read timeout); the MCP reconnect supervisor
/// uses this directly.
#[derive(Debug, Clone, Copy)]
pub struct BackoffPolicy {
    /// Total tries, initial attempt included.
    pub max_attempts: u32,
    /// Wait before the first retry; doubles each retry.
    pub base_delay_ms: u64,
    /// Ceiling for the doubling.
    pub max_delay_ms: u64,
}

impl Default for BackoffPolicy {
    /// dsh `RECONNECT_DEFAULTS`: 500ms → 30s, 10 attempts.
    fn default() -> Self {
        Self {
            max_attempts: 10,
            base_delay_ms: 500,
            max_delay_ms: 30_000,
        }
    }
}

/// Delay before `attempt` (1-based, so the first retry waits `base`),
/// capped, with a server-supplied `Retry-After` override when present.
#[must_use]
pub fn delay(attempt: u32, retry_after_ms: Option<u64>, policy: &BackoffPolicy) -> Duration {
    let ms = match retry_after_ms {
        Some(ra) => ra.min(policy.max_delay_ms),
        None => policy
            .base_delay_ms
            .saturating_mul(1u64 << (attempt - 1).min(16))
            .min(policy.max_delay_ms),
    };
    Duration::from_millis(ms)
}
