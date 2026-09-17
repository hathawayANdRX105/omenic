//! Unit tests for [`mcp::HttpTransport`] and [`mcp::McpReconnect`].
//!
//! No network access: the retry layer is driven by a fake in-memory
//! transport that fails N times and then succeeds.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use mcp::http::DEFAULT_TIMEOUT_MS;
use mcp::{HttpTransport, McpError, McpReconnect, McpTransport, ReconnectPolicy};

/// A fake transport that fails its first `n_failures` calls with a
/// transport error, then succeeds forever after.
struct FlakyTransport {
    next: AtomicU64,
    failures: Mutex<usize>,
}

impl FlakyTransport {
    fn new(failures: usize) -> Self {
        Self {
            next: AtomicU64::new(1),
            failures: Mutex::new(failures),
        }
    }

    /// Fail one call; idempotently clamps at zero.
    fn take_failure(&self) -> bool {
        let mut f = *self.failures.lock().unwrap();
        if f > 0 {
            f -= 1;
            *self.failures.lock().unwrap() = f;
            true
        } else {
            false
        }
    }
}

impl McpTransport for FlakyTransport {
    fn next_id(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    fn notify(&self, _line: &str) -> Result<(), McpError> {
        if self.take_failure() {
            Err(McpError::Transport("flaky: pipe closed".into()))
        } else {
            Ok(())
        }
    }

    fn roundtrip(&self, _id: u64, _line: &str, _signal: &AtomicBool) -> Result<String, McpError> {
        if self.take_failure() {
            Err(McpError::Transport("flaky: server closed stdout".into()))
        } else {
            Ok(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#.into())
        }
    }
}

#[test]
fn http_transport_new_and_next_id() {
    let t = HttpTransport::new("http://localhost:9999/mcp".into(), 1000);
    assert_eq!(t.url(), "http://localhost:9999/mcp");
    let (a, b, c) = (t.next_id(), t.next_id(), t.next_id());
    assert_eq!((a, b, c), (1, 2, 3));
}

#[test]
fn default_timeout_matches_mcp_timeout() {
    // MCP_TIMEOUT is 30s; the default ms value must mirror it.
    assert_eq!(DEFAULT_TIMEOUT_MS, 30_000);
}

#[test]
fn reconnect_policy_default_values() {
    let p = ReconnectPolicy::default();
    assert_eq!(p.initial_delay_ms, 500);
    assert_eq!(p.max_delay_ms, 30_000);
    assert_eq!(p.max_attempts, 10);
}

#[test]
fn reconnect_retries_transport_errors_then_succeeds() {
    // Fails twice, succeeds on the third call.
    let inner = Arc::new(FlakyTransport::new(2));
    let rc = McpReconnect::new(inner.clone(), ReconnectPolicy::default());
    let signal = AtomicBool::new(false);
    let out = rc
        .roundtrip(1, "{}", &signal)
        .expect("succeeds after retries");
    assert!(out.contains("\"result\""));
    // All flaky failures were consumed.
    assert_eq!(*inner.failures.lock().unwrap(), 0);
}

#[test]
fn reconnect_gives_up_after_max_attempts() {
    // Fails forever, and a policy capped at 3 attempts must stop after 3.
    let inner = Arc::new(FlakyTransport::new(usize::MAX));
    let rc = McpReconnect::new(
        inner,
        ReconnectPolicy {
            initial_delay_ms: 1,
            max_delay_ms: 2,
            max_attempts: 3,
        },
    );
    let signal = AtomicBool::new(false);
    let err = rc
        .roundtrip(1, "{}", &signal)
        .expect_err("must fail after max attempts");
    match err {
        McpError::Transport(m) => assert!(m.contains("flaky")),
        other => panic!("expected Transport error, got {other:?}"),
    }
}

#[test]
fn reconnect_notify_retries_then_succeeds() {
    let inner = Arc::new(FlakyTransport::new(1));
    let rc = McpReconnect::new(inner, ReconnectPolicy::default());
    rc.notify("n").expect("notify succeeds after one retry");
}
