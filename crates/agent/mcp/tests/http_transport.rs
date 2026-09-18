//! Unit tests for [`mcp::HttpTransport`] and [`mcp::McpReconnect`].
//!
//! No network access: the retry layer is driven by a fake in-memory
//! transport that fails N times and then succeeds.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

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

// ---------------------------------------------------------------------------
// HttpTransport against a real TCP mock: HTTP status classification
// ---------------------------------------------------------------------------

/// A one-shot HTTP mock: answers the first request with `status` and
/// `body`, then serves nothing. Shape mirrors
/// `crates/agent/orbit/tests/fallback_waterfall.rs`'s `MockProvider`.
struct MockHttp {
    addr: String,
}

impl MockHttp {
    fn start(status: u16, reason: &str, body: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
        let addr = listener.local_addr().expect("local addr");
        let resp = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len(),
        );
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let resp = resp.clone();
                thread::spawn(move || {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    // Drain the request head so the client can send, then
                    // answer once and close.
                    let mut buf = [0u8; 4096];
                    let _ = stream.read(&mut buf);
                    let _ = stream.write_all(resp.as_bytes());
                    let _ = stream.flush();
                });
            }
        });
        MockHttp {
            addr: format!("http://127.0.0.1:{}", addr.port()),
        }
    }
}

/// A 4xx (other than 408/429) is the server's verdict on this exact
/// request — bad URL, auth, unknown endpoint — so it must surface as a
/// non-retryable [`McpError::Server`] carrying the status code and a body
/// excerpt. Pin: `McpReconnect::is_retryable` rejects it, so no retry loop
/// burns the budget on it.
#[test]
fn http_4xx_is_non_retryable_server_error_with_body() {
    let m = MockHttp::start(404, "Not Found", "{\"error\":\"unknown endpoint\"}");
    let t = HttpTransport::new(m.addr, 5000);
    let signal = AtomicBool::new(false);
    let err = t
        .roundtrip(1, "{\"jsonrpc\":\"2.0\"}", &signal)
        .expect_err("404 must fail");
    match &err {
        McpError::Server { code, message } => {
            assert_eq!(*code, 404);
            assert!(
                message.contains("unknown endpoint"),
                "body excerpt must be carried in the message, got {message:?}"
            );
        }
        other => panic!("expected McpError::Server, got {other:?}"),
    }
    // Wrapping in the reconnect supervisor must not retry it either.
    let m2 = MockHttp::start(404, "Not Found", "nope");
    let inner = Arc::new(HttpTransport::new(m2.addr, 5000));
    let rc = McpReconnect::new(
        inner,
        ReconnectPolicy {
            initial_delay_ms: 1,
            max_delay_ms: 2,
            max_attempts: 5,
        },
    );
    let err2 = rc
        .roundtrip(1, "{}", &signal)
        .expect_err("404 must fail without retries");
    assert!(
        matches!(err2, McpError::Server { .. }),
        "supervisor must pass a 4xx through as non-retryable, got {err2:?}"
    );
}

/// A 5xx is a transport-level blip the server may recover from: it stays
/// [`McpError::Transport`] and therefore retryable — the supervisor burns
/// its attempts before surfacing the last error.
#[test]
fn http_5xx_stays_retryable_transport_error() {
    let m = MockHttp::start(500, "Internal Server Error", "boom");
    let inner = Arc::new(HttpTransport::new(m.addr, 5000));
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
        .expect_err("500 with no recovery must fail");
    assert!(
        matches!(&err, McpError::Transport(msg) if msg.contains("500")),
        "expected retryable Transport(\"http 500\"), got {err:?}"
    );
}
