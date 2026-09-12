//! Retry-behaviour integration tests against a local mock server:
//! 429 then success, no retry on client errors, and a stream that dies
//! before producing any text.

use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use adaptor::openai::{RetryPolicy, stream_cb_with_policy};
use adaptor::{Context, Model, StopReason, StreamEvent};

const FAST_POLICY: RetryPolicy = RetryPolicy {
    max_attempts: 4,
    base_delay_ms: 1,
    max_delay_ms: 4,
};

/// A server that answers each connection with scripted raw responses and
/// counts how many connections it served.
fn serve_scripted(responses: Vec<String>) -> (u16, Arc<AtomicUsize>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let served = Arc::new(AtomicUsize::new(0));
    let served_clone = Arc::clone(&served);
    std::thread::spawn(move || {
        // Ignore accept errors once the listener is dropped at test end.
        for resp in responses {
            let Ok((mut sock, _)) = listener.accept() else {
                break;
            };
            served_clone.fetch_add(1, Ordering::SeqCst);
            let mut buf = [0u8; 4096];
            let _ = sock.read(&mut buf);
            if sock.write_all(resp.as_bytes()).is_err() {
                break;
            }
            let _ = sock.flush();
            // Dropping a socket that still holds unread request data makes
            // the kernel send RST, which would clobber the response the
            // client is about to read. Drain with a short timeout instead.
            let _ = sock.set_read_timeout(Some(std::time::Duration::from_millis(50)));
            let mut drain = [0u8; 4096];
            while let Ok(n) = sock.read(&mut drain) {
                if n == 0 {
                    break;
                }
            }
        }
    });
    (port, served)
}

fn model(port: u16) -> Model {
    Model {
        api_key: "k".into(),
        model: "m".into(),
        base_url: Some(format!("http://127.0.0.1:{port}/v1")),
        max_tokens: None,
    }
}

fn sse_ok() -> String {
    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n\
                data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n";
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{}",
        body.len(),
        body
    )
}

fn status_line(status: &str, body: &str, extra: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\ncontent-length: {}\r\n{extra}\r\n\r\n{body}",
        body.len()
    )
}

fn run(port: u16, policy: RetryPolicy) -> Vec<StreamEvent> {
    let mut received = Vec::new();
    stream_cb_with_policy(
        &model(port),
        &Context::default(),
        &[],
        &AtomicBool::new(false),
        &mut |ev| received.push(ev.clone()),
        policy,
    );
    received
}

/// A thin connection was accepted but closed without a status line: treated
/// by ureq as a transport failure. Retryable before any delta, and the
/// second attempt succeeds.
#[test]
fn dead_connection_retried_then_success() {
    let (port, served) = serve_scripted(vec![
        // Accept and immediately drop: no response bytes at all.
        String::new(),
        sse_ok(),
    ]);
    let events = run(port, FAST_POLICY);
    assert_eq!(served.load(Ordering::SeqCst), 2, "exactly one retry");
    assert_eq!(
        events,
        vec![
            StreamEvent::TextDelta("ok".into()),
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn
            },
        ],
        "replayed call must produce a single clean copy of the stream"
    );
}

/// 429 then success: the provider said "slow down", we did, and the retry
/// carries the full event sequence with no duplicated text.
#[test]
fn rate_limit_retried_then_success() {
    let (port, served) = serve_scripted(vec![
        status_line("429 Too Many Requests", "rate hit!", ""),
        sse_ok(),
    ]);
    let events = run(port, FAST_POLICY);
    assert_eq!(served.load(Ordering::SeqCst), 2);
    assert_eq!(
        events,
        vec![
            StreamEvent::TextDelta("ok".into()),
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn
            },
        ]
    );
}

/// 400 is a deterministic client error: no retry, single connection, one
/// error event.
#[test]
fn client_error_not_retried() {
    let (port, served) = serve_scripted(vec![status_line("400 Bad Request", "bad key", "")]);
    let events = run(port, FAST_POLICY);
    assert_eq!(served.load(Ordering::SeqCst), 1, "must not retry 4xx");
    match &events[..] {
        [StreamEvent::Error(e)] => assert!(e.contains("400"), "{e}"),
        other => panic!("expected single error event, got {other:?}"),
    }
}

/// 5xx keeps failing until attempts are exhausted, then one error event.
#[test]
fn exhausted_retries_surface_single_error() {
    // connection: close so each attempt opens a fresh connection instead of
    // reusing a pooled socket this one-shot server already dropped.
    let close = "connection: close\r\n";
    let (port, served) = serve_scripted(vec![
        status_line("500 Internal Server Error", "bad", close),
        status_line("500 Internal Server Error", "bad", close),
        status_line("500 Internal Server Error", "bad", close),
        status_line("500 Internal Server Error", "bad", close),
    ]);
    // Four scripted failures for four attempts; the surfaced error is the
    // last one, still the provider's 500.
    let events = run(port, FAST_POLICY);
    assert_eq!(
        served.load(Ordering::SeqCst),
        4,
        "every attempt hit the server"
    );
    match &events[..] {
        [StreamEvent::Error(e)] => assert!(e.contains("500"), "{e}"),
        other => panic!("expected single error event, got {other:?}"),
    }
}

/// A `Retry-After` header must be honored over the default backoff. We can't
/// observe the sleep directly here, but a large value must still be capped
/// by `max_delay_ms` — keep the policy ceiling tiny and confirm the run
/// finishes quickly (well under the raw header value).
#[test]
fn retry_after_header_capped_by_policy() {
    let (port, _served) = serve_scripted(vec![
        status_line("429 Too Many Requests", "slow", "retry-after: 3600\r\n"),
        sse_ok(),
    ]);
    let policy = RetryPolicy {
        max_attempts: 4,
        base_delay_ms: 1,
        max_delay_ms: 20,
    };
    let started = std::time::Instant::now();
    let events = run(port, policy);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "retry-after 3600s must be capped by max_delay_ms"
    );
    assert_eq!(events.len(), 2);
}

/// Abort during the backoff sleep: no further attempts, Done(Aborted)
/// arrives promptly instead of after the full (60s) backoff.
#[test]
fn abort_during_backoff_stops_the_loop() {
    let (port, _served) = serve_scripted(vec![
        status_line("500 Internal Server Error", "bad", ""),
        sse_ok(),
    ]);

    let policy = RetryPolicy {
        max_attempts: 4,
        base_delay_ms: 60_000,
        max_delay_ms: 60_000,
    };
    let signal = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let signal2 = Arc::clone(&signal);
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(120));
        signal2.store(true, Ordering::Relaxed);
    });

    let (tx, rx) = std::sync::mpsc::channel::<StreamEvent>();
    let signal3 = Arc::clone(&signal);
    std::thread::spawn(move || {
        stream_cb_with_policy(
            &model(port),
            &Context::default(),
            &[],
            &signal3,
            &mut |ev| {
                tx.send(ev.clone()).unwrap();
            },
            policy,
        );
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let first = loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        assert!(left > std::time::Duration::ZERO, "abort never surfaced");
        match rx.recv_timeout(left.min(std::time::Duration::from_millis(100))) {
            Ok(ev) => break ev,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("stream_cb thread died without a terminal event")
            }
        }
    };
    assert!(
        matches!(
            &first,
            StreamEvent::Done {
                stop_reason: StopReason::Aborted
            }
        ),
        "backoff sleep must yield to the abort signal: {first:?}"
    );
}

// ===== policy unit tests (pure functions) =====

#[test]
fn retryable_status_classification() {
    use adaptor::openai::is_retryable_status;
    assert!(is_retryable_status(429));
    assert!(is_retryable_status(500));
    assert!(is_retryable_status(503));
    assert!(!is_retryable_status(400));
    assert!(!is_retryable_status(401));
    assert!(!is_retryable_status(404));
}

#[test]
fn retryable_transport_classification() {
    use adaptor::openai::is_retryable_transport;
    assert!(is_retryable_transport(ureq::ErrorKind::ConnectionFailed));
    assert!(is_retryable_transport(ureq::ErrorKind::Dns));
    assert!(is_retryable_transport(ureq::ErrorKind::Io));
    assert!(!is_retryable_transport(ureq::ErrorKind::InvalidUrl));
}

#[test]
fn backoff_doubles_and_caps() {
    use adaptor::openai::backoff_delay;
    let policy = RetryPolicy {
        max_attempts: 4,
        base_delay_ms: 500,
        max_delay_ms: 10_000,
    };
    assert_eq!(backoff_delay(1, None, &policy), Duration::from_millis(500));
    assert_eq!(backoff_delay(2, None, &policy), Duration::from_millis(1000));
    assert_eq!(backoff_delay(3, None, &policy), Duration::from_millis(2000));
    assert_eq!(backoff_delay(4, None, &policy), Duration::from_millis(4000));
}

#[test]
fn retry_after_overrides_backoff_and_caps() {
    use adaptor::openai::backoff_delay;
    let policy = RetryPolicy::default();
    // retry-after arrives pre-converted to milliseconds.
    assert_eq!(
        backoff_delay(1, Some(3000), &policy),
        Duration::from_millis(3000)
    );
    // A hostile Retry-After (1h) must not wedge the loop.
    assert_eq!(
        backoff_delay(1, Some(3_600_000), &policy),
        Duration::from_millis(10_000)
    );
}
