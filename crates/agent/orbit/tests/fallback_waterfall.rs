//! B2b1 — waterfall LLM fallback across configured providers.
//!
//! The observable contract: when the primary provider fails *before
//! emitting any content* (terminal `Error`, no delta/tool-call leaked),
//! the next configured provider takes over; a partially-emitted turn is
//! never replayed on another provider; when every provider fails before
//! emitting anything, one terminal `Error` names the last provider.
//!
//! Two `std::net::TcpListener` mocks stand in for the providers (shape
//! mirrors `crates/infra/daemon/tests/session_resume.rs`'s `MockOpenAi`):
//! each counts its `/chat/completions` requests so the test can assert
//! the per-provider retry budget and the waterfall hand-off.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use adaptor::{Context, Message, Model, StopReason, StreamEvent, openai::RetryPolicy};
use orbit::{LlmBackend, LlmProvider, WaterfallLlm};

// ---------------------------------------------------------------------------
// mock
// ---------------------------------------------------------------------------

/// One OpenAI-compatible mock endpoint. Per-connection behaviour:
/// * `fail_n` connections (in arrival order) → HTTP 500 (retryable),
/// * if `first_delta_then_dry`, the first *successful* connection sends
///   one text delta and then the socket is dropped mid-stream (stream read
///   fails *after* content leaked),
/// * otherwise → one clean SSE text stream (delta + `finish_reason`).
///
/// `/chat/completions` requests are counted so the waterfall hand-off and
/// the per-provider retry budget are assertable.
struct MockProvider {
    addr: String,
    hits: Arc<AtomicUsize>,
}

impl MockProvider {
    fn start(fail_n: usize, first_delta_then_dry: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
        let addr = listener.local_addr().expect("local addr");
        let hits = Arc::new(AtomicUsize::new(0));
        let conn_seq = Arc::new(AtomicUsize::new(0));
        let dry_once = Arc::new(Mutex::new(first_delta_then_dry));

        let hits_srv = Arc::clone(&hits);
        let seq_srv = Arc::clone(&conn_seq);
        let dry_srv = Arc::clone(&dry_once);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let hits = Arc::clone(&hits_srv);
                let seq = Arc::clone(&seq_srv);
                let dry = Arc::clone(&dry_srv);
                thread::spawn(move || {
                    hits.fetch_add(1, Ordering::SeqCst);
                    let n = seq.fetch_add(1, Ordering::SeqCst);
                    if n < fail_n {
                        // 500 before reading anything: the connection is
                        // closed after the head is flushed.
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                        let _ = stream.write_all(
                            b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 11\r\nConnection: close\r\n\r\nboom-b2b150",
                        );
                        let _ = stream.flush();
                        return;
                    }
                    let dry_this_conn = {
                        let mut g = dry.lock().expect("dry lock");
                        let v = *g;
                        *g = false;
                        v
                    };
                    if dry_this_conn {
                        read_request(&mut stream);
                        let delta = sse_delta("part");
                        let _ = stream.write_all(delta.as_bytes());
                        let _ = stream.flush();
                        // Drop the socket mid-stream: the next read on the
                        // client side fails, *after* the delta leaked.
                        return;
                    }
                    serve_clean(stream);
                });
            }
        });

        MockProvider {
            addr: format!("http://127.0.0.1:{}", addr.port()),
            hits,
        }
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

/// Read one HTTP request (headers until `\r\n\r\n`, then the body).
/// Returns the raw head bytes; request bodies are not inspected here.
fn read_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1];
    while let Ok(n) = stream.read(&mut tmp) {
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let head = String::from_utf8_lossy(&buf).to_string();
    let body_len = head
        .split("\r\n")
        .find_map(|l| {
            l.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .and_then(|v| v.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    while buf.len() < head.len() + body_len {
        let mut chunk = vec![0u8; head.len() + body_len - buf.len()];
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
    }
    buf
}

/// Read the request, then reply with one clean SSE text stream and close.
fn serve_clean(stream: TcpStream) {
    let mut stream = stream;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = read_request(&mut stream);
    let reply = sse_delta("ok") + &sse_finish();
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        reply.len(),
        reply,
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

/// One SSE `data:` line carrying a text delta.
fn sse_delta(delta: &str) -> String {
    format!(
        "data: {}\n\n",
        serde_json::json!({
            "choices": [{
                "delta": { "content": delta },
                "finish_reason": null
            }]
        })
    )
}

/// The stream-closing SSE line: a `finish_reason` chunk, nothing after.
fn sse_finish() -> String {
    format!(
        "data: {}\n\n",
        serde_json::json!({
            "choices": [{
                "delta": { "content": "" },
                "finish_reason": "stop"
            }]
        })
    )
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn provider(addr: &str, name: &str) -> LlmProvider {
    LlmProvider {
        api_key: "test-key".into(),
        model: name.to_string(),
        base_url: Some(addr.to_string()),
        max_tokens: None,
    }
}

fn run_once(backend: &dyn LlmBackend) -> Vec<StreamEvent> {
    let signal = std::sync::atomic::AtomicBool::new(false);
    let ctx = Context {
        system_prompt: None,
        messages: vec![Message::user_text("hi")],
    };
    let events = backend.stream(
        &Model {
            api_key: "test-key".into(),
            model: "mock".into(),
            base_url: None,
            max_tokens: None,
        },
        &ctx,
        &[],
        &signal,
    );
    events
}

fn terminal(events: &[StreamEvent]) -> &StreamEvent {
    events.last().expect("stream is never empty")
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

/// Primary keeps failing (500, retryable) up to `RetryPolicy.max_attempts`,
/// then the fallback serves a clean stream. The terminal event is the
/// fallback's `Done`; the primary mock saw exactly `max_attempts` requests
/// (per-provider retry budget, not a waterfall loop) and the fallback saw
/// 1.
#[test]
fn waterfall_drops_to_fallback_after_retries_exhausted() {
    let policy = RetryPolicy {
        max_attempts: 2,
        base_delay_ms: 1,
        max_delay_ms: 2,
    };
    let a = MockProvider::start(usize::MAX, false); // always 500
    let b = MockProvider::start(0, false); // always clean
    let wf = WaterfallLlm {
        primary: provider(&a.addr, "primary"),
        fallbacks: vec![provider(&b.addr, "fallback")],
        retry: policy,
    };
    let events = run_once(&wf);
    assert!(
        matches!(
            terminal(&events),
            StreamEvent::Done { stop_reason } if *stop_reason == StopReason::EndTurn
        ),
        "terminal must be the fallback's Done, saw {events:?}"
    );
    assert!(
        a.hits() == 2,
        "primary mock must see max_attempts=2 requests (per-provider retry), saw {}",
        a.hits()
    );
    assert!(
        b.hits() == 1,
        "fallback mock must see exactly 1 request, saw {}",
        b.hits()
    );
}

/// A provider that emits one text delta before its stream dies must NOT
/// hand off to the next provider — the consumer already saw content, so a
/// replay would duplicate. The round ends in that provider's `Error`, and
/// the fallback is never touched.
#[test]
fn no_switch_after_content_leaked() {
    let policy = RetryPolicy {
        max_attempts: 2,
        base_delay_ms: 1,
        max_delay_ms: 2,
    };
    let a = MockProvider::start(0, true); // delta then mid-stream drop
    let b = MockProvider::start(0, false);
    let wf = WaterfallLlm {
        primary: provider(&a.addr, "primary"),
        fallbacks: vec![provider(&b.addr, "fallback")],
        retry: policy,
    };
    let events = run_once(&wf);
    assert!(
        matches!(terminal(&events), StreamEvent::Error(_)),
        "terminal must be an Error (content leaked, no replay), saw {events:?}"
    );
    assert!(
        b.hits() == 0,
        "fallback must not be touched after content leaked, saw {}",
        b.hits()
    );
}

/// Every provider fails before emitting anything → one terminal `Error`
/// naming the last provider's index. Each mock sees `max_attempts`
/// requests.
#[test]
fn all_providers_fail_yields_indexed_error() {
    let policy = RetryPolicy {
        max_attempts: 1,
        base_delay_ms: 1,
        max_delay_ms: 2,
    };
    let a = MockProvider::start(usize::MAX, false);
    let b = MockProvider::start(usize::MAX, false);
    let wf = WaterfallLlm {
        primary: provider(&a.addr, "primary"),
        fallbacks: vec![provider(&b.addr, "fallback")],
        retry: policy,
    };
    let events = run_once(&wf);
    let StreamEvent::Error(err) = terminal(&events) else {
        panic!("terminal must be a single Error, saw {events:?}");
    };
    assert!(
        err.contains("provider 1"),
        "error must name the last provider index, got {err:?}"
    );
    assert!(err.contains("all providers exhausted"), "got {err:?}");
}

/// `WaterfallLlm::solo` against a clean provider: one request, one `Done`,
/// no fallbacks — the "no fallbacks configured" path.
#[test]
fn solo_behaves_like_historic_single_provider() {
    let b = MockProvider::start(0, false);
    let wf = WaterfallLlm::solo(provider(&b.addr, "solo"));
    let events = run_once(&wf);
    assert!(
        matches!(
            terminal(&events),
            StreamEvent::Done { stop_reason } if *stop_reason == StopReason::EndTurn
        ),
        "saw {events:?}"
    );
    assert!(b.hits() == 1, "saw {}", b.hits());
}

/// The `LlmBackend` object-safety check: a `WaterfallLlm` behind
/// `Arc<dyn LlmBackend + Send + Sync>` (the daemon's `OrbitSetup` seam)
/// runs the waterfall exactly as the direct calls above.
#[test]
fn waterfall_through_dyn_backend() {
    let a = MockProvider::start(usize::MAX, false);
    let b = MockProvider::start(0, false);
    let wf = Arc::new(WaterfallLlm {
        primary: provider(&a.addr, "primary"),
        fallbacks: vec![provider(&b.addr, "fallback")],
        retry: RetryPolicy {
            max_attempts: 1,
            base_delay_ms: 1,
            max_delay_ms: 2,
        },
    });
    let backend: Arc<dyn LlmBackend + Send + Sync> = wf;
    let events = run_once(backend.as_ref());
    assert!(
        matches!(
            terminal(&events),
            StreamEvent::Done { stop_reason } if *stop_reason == StopReason::EndTurn
        ),
        "saw {events:?}"
    );
    assert!(a.hits() == 1 && b.hits() == 1);
}
