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

use crate::{McpError, McpTransport};

/// Backoff parameters for [`McpReconnect`]. The delay curve itself lives in
/// [`llm::backoff`] — the LLM call path shares it — so the two retry loops
/// cannot drift apart.
pub type ReconnectPolicy = llm::backoff::BackoffPolicy;

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
        self.retry(|_| self.inner.notify(line), "no attempt made")
    }

    fn roundtrip(&self, id: u64, line: &str, signal: &AtomicBool) -> Result<String, McpError> {
        self.retry(
            |_| self.inner.roundtrip(id, line, signal),
            "no attempt made",
        )
    }
}

impl McpReconnect {
    /// The retry walk both transport methods share: attempt, sleep the
    /// backoff curve between attempts, and give up immediately on an error
    /// that isn't a dead link.
    ///
    /// `fallback` is the error surfaced when `max_attempts` is 0 — the loop
    /// never runs, so there is no real failure to report.
    fn retry<T, F>(&self, mut op: F, fallback: &str) -> Result<T, McpError>
    where
        F: FnMut(u32) -> Result<T, McpError>,
    {
        let mut last_err = McpError::Transport(fallback.into());
        for attempt in 1..=self.policy.max_attempts {
            if attempt > 1 {
                // The caller's abort signal does not reach the sleep, so the
                // wait is only bounded, not cancellable. Adding a signal to
                // `notify` is a public trait change — defer until a caller
                // needs it.
                sleep(llm::backoff::delay(attempt - 1, None, &self.policy));
            }
            match op(attempt) {
                Ok(value) => return Ok(value),
                Err(e) if Self::is_retryable(&e) => last_err = e,
                Err(e) => return Err(e),
            }
        }
        Err(last_err)
    }
}
