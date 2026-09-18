//! Streamable-HTTP MCP transport: JSON-RPC over a single HTTP POST per request.
//!
//! The wire protocol is the same line-delimited JSON-RPC 2.0 that
//! [`super::request_line`] and [`super::parse_response`] already handle.
//! Each request is a `POST` to the server's base URL; the response body is a
//! single JSON line. This is not SSE streaming — it is one request/response
//! pair, matching how the stdio transport works (send a line, read a line back).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use ureq::Agent;

use crate::{MCP_TIMEOUT, McpError, McpTransport};

/// Default per-call timeout in ms, used when no `tool_call_timeout_ms` is set.
pub const DEFAULT_TIMEOUT_MS: u64 = MCP_TIMEOUT.as_millis() as u64;

/// HTTP transport: one blocking `POST` per request, response body is one JSON line.
pub struct HttpTransport {
    url: String,
    next: AtomicU64,
    timeout_ms: u64,
}

impl HttpTransport {
    /// New transport posting to `url` with a per-call timeout in ms.
    pub fn new(url: String, timeout_ms: u64) -> Self {
        Self {
            url,
            next: AtomicU64::new(1),
            timeout_ms,
        }
    }

    /// The server's base URL.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// A ureq agent with this transport's per-call timeouts. The global
    /// `timeout` bounds the whole request (connect + read); a stalled body
    /// read is its own separate budget.
    fn client(&self) -> Agent {
        ureq::AgentBuilder::new()
            .timeout(Duration::from_millis(self.timeout_ms))
            .timeout_read(Duration::from_millis(self.timeout_ms))
            .build()
    }

    /// POST `line`, mapping transport errors to [`McpError::Transport`].
    ///
    /// Returns the response body (the JSON-RPC response line) on any 2xx.
    fn post(&self, line: &str) -> Result<String, McpError> {
        let res = self
            .client()
            .post(&self.url)
            .send_string(line)
            .map_err(|e| {
                match e {
                    // ureq reports timeouts and connection failures alike here.
                    // `McpReconnect` treats any `Transport` as retryable, so the
                    // distinction stays at the supervisor layer, not this one.
                    ureq::Error::Status(code, _) => McpError::Transport(format!("http {code}")),
                    other => McpError::Transport(format!("http {other}")),
                }
            })?;
        let body = res
            .into_string()
            .map_err(|e| McpError::Transport(format!("reading response body: {e}")))?;
        Ok(body)
    }
}

impl McpTransport for HttpTransport {
    fn next_id(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    fn notify(&self, line: &str) -> Result<(), McpError> {
        // Notifications carry no `id`; the server answers `202 Accepted` with
        // an empty body. Fire and discard the reply — a failed POST is still a
        // transport error the caller should surface.
        let _ = self.post(line)?;
        Ok(())
    }

    fn roundtrip(&self, _id: u64, line: &str, signal: &AtomicBool) -> Result<String, McpError> {
        if signal.load(Ordering::Relaxed) {
            return Err(McpError::Aborted);
        }
        let body = self.post(line)?;
        if signal.load(Ordering::Relaxed) {
            return Err(McpError::Aborted);
        }
        // Same contract as the stdio transport: hand back the raw response line
        // so the caller can `parse_response` it. An empty body is not valid
        // JSON-RPC — say so explicitly instead of failing with a bare parse error.
        if body.trim().is_empty() {
            return Err(McpError::Transport("empty response body".into()));
        }
        Ok(body)
    }
}
