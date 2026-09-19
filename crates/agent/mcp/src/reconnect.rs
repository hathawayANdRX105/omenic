//! Exponential-backoff reconnect supervisor for MCP connections.
//!
//! `McpReconnect` is a *transparent retry layer*: it retries the same
//! transport on transport-level failures (`Transport`) with exponential
//! backoff. It does **not** know how to re-establish a new connection
//! (no re-spawn logic) — that wiring is the daemon's job (T3). The
//! practical effect: a hung stdio child whose pipes have closed returns
//! `Transport("server closed stdout")` on every retry and the wrapper gives
//! up after `max_attempts`, surfacing the last error.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::thread::sleep;
use std::time::Duration;

use crate::{McpError, McpTransport};

/// Backoff parameters for [`McpReconnect`].
#[derive(Debug, Clone)]
pub struct ReconnectPolicy {
    /// Delay before the first retry.
    pub initial_delay_ms: u64,
    /// Upper bound on the per-attempt delay (doubling stops here).
    pub max_delay_ms: u64,
    /// Total attempts (initial + retries) before giving up.
    pub max_attempts: u32,
}

impl Default for ReconnectPolicy {
    /// Same defaults as dsh `RECONNECT_DEFAULTS`: 500ms → 30s, 10 attempts.
    fn default() -> Self {
        Self {
            initial_delay_ms: 500,
            max_delay_ms: 30_000,
            max_attempts: 10,
        }
    }
}

/// Retry wrapper around any [`McpTransport`].
pub struct McpReconnect {
    inner: Arc<dyn McpTransport>,
    policy: ReconnectPolicy,
}

impl McpReconnect {
    pub fn new(transport: Arc<dyn McpTransport>, policy: ReconnectPolicy) -> Self {
        Self {
            inner: transport,
            policy,
        }
    }

    /// The wrapped transport, for inspection (tests, T3 daemon wiring).
    pub fn inner(&self) -> &Arc<dyn McpTransport> {
        &self.inner
    }

    /// The policy in effect.
    pub fn policy(&self) -> &ReconnectPolicy {
        &self.policy
    }

    /// True for errors worth a retry: the link is dead, not the request.
    ///
    /// `Timeout` is deliberately *not* retryable: the request already burned
    /// its whole 30s budget, so 10 attempts would stretch one call to
    /// ~5 minutes; and a stdio re-send of the same id risks the server
    /// executing the request twice (the first attempt may still land after
    /// the client gave up waiting). A dead link (`Transport`) fails fast and
    /// identically on retry, so only that is worth the backoff.
    ///
    /// Scope: this makes the `Timeout` *variant* non-retryable. It does not
    /// remove the retry budget from HTTP calls — the http transport maps its
    /// own timeouts to `Transport` (see `http::HttpTransport::post`), so a
    /// stalled HTTP request still retries through this supervisor.
    ///
    // ponytail: `notify` takes no abort signal, so a retry loop inside
    // `notify` cannot be interrupted mid-backoff (the sleep is capped, not
    // cancellable). Adding a signal parameter to `notify` is a public trait
    // change — defer until a caller actually needs it.
    fn is_retryable(e: &McpError) -> bool {
        matches!(e, McpError::Transport(_))
    }
}

impl McpTransport for McpReconnect {
    fn next_id(&self) -> u64 {
        self.inner.next_id()
    }

    fn notify(&self, line: &str) -> Result<(), McpError> {
        let mut delay = Duration::from_millis(self.policy.initial_delay_ms);
        let max = Duration::from_millis(self.policy.max_delay_ms);
        let mut last_err = McpError::Transport("no attempt made".into());
        for attempt in 0..self.policy.max_attempts {
            if attempt > 0 {
                // Abort during backoff: the caller's signal won't reach us here
                // (notify has no signal), so just finish the sleep — it is capped.
                sleep(delay);
                delay = (delay * 2).min(max);
            }
            match self.inner.notify(line) {
                Ok(()) => return Ok(()),
                Err(e) if Self::is_retryable(&e) => {
                    last_err = e;
                    if attempt + 1 < self.policy.max_attempts {
                        continue;
                    }
                    return Err(last_err);
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err)
    }

    fn roundtrip(&self, id: u64, line: &str, signal: &AtomicBool) -> Result<String, McpError> {
        let mut delay = Duration::from_millis(self.policy.initial_delay_ms);
        let max = Duration::from_millis(self.policy.max_delay_ms);
        let mut last_err = McpError::Transport("no attempt made".into());
        for attempt in 0..self.policy.max_attempts {
            if attempt > 0 {
                // Back off between retries, polling the abort signal so a hung
                // retry loop can still be interrupted.
                sleep(delay);
                delay = (delay * 2).min(max);
            }
            match self.inner.roundtrip(id, line, signal) {
                Ok(resp) => return Ok(resp),
                Err(e) if Self::is_retryable(&e) => {
                    last_err = e;
                    if attempt + 1 < self.policy.max_attempts {
                        continue;
                    }
                    return Err(last_err);
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err)
    }
}
