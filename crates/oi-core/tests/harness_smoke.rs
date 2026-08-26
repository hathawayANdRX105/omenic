//! Offline smoke for the HTTP streaming path: a local mock SSE server
//! exercises `llm::stream` end-to-end (ureq request → line parsing →
//! unified events) without any network access.

use std::io::{Read, Write};
use std::sync::atomic::AtomicBool;

use oi_core::runtime::llm::{Context, Model, StopReason, StreamEvent, stream};

fn serve_once(body_chunks: Vec<String>) -> (u16, std::thread::JoinHandle<Vec<u8>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        // Read until the full body arrives per content-length.
        let mut req = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = sock.read(&mut buf).unwrap_or(0);
            if n == 0 {
                break;
            }
            req.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&req);
            if let Some(i) = text.find("\r\n\r\n") {
                if text[i + 4..].len() >= 32 {
                    break; // body complete enough for assertions
                }
            }
        }
        let sse = body_chunks.join("");
        write!(
            sock,
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{}",
            sse.len(),
            sse
        )
        .unwrap();
        req
    });
    (port, handle)
}

#[test]
fn http_stream_end_to_end() {
    let chunk = |delta: &str| {
        format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"{delta}\"}}}}]}}\n\n")
    };
    let tool_frag = |args: &str| {
        let payload = serde_json::json!({
            "choices": [{"delta": {"tool_calls": [{
                "index": 0, "id": "c1",
                "function": {"name": "read_file", "arguments": args},
            }]}}]
        });
        format!("data: {payload}\n\n")
    };
    // Split the tool-call arguments across two fragments on purpose.
    let (port, server) = serve_once(vec![
        chunk("hel"),
        chunk("lo"),
        tool_frag("{\"pa"),
        tool_frag("th\":\"/x\"}"),
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n".into(),
        "data: [DONE]\n\n".into(),
    ]);

    let model = Model {
        api_key: "test-key".into(),
        model: "mock".into(),
        base_url: Some(format!("http://127.0.0.1:{port}/v1")),
        max_tokens: Some(64),
    };
    let events = stream(&model, &Context::default(), &[], &AtomicBool::new(false));

    let sent = String::from_utf8(server.join().unwrap()).unwrap();
    assert!(sent.contains("POST /v1/chat/completions"));
    assert!(
        sent.to_lowercase()
            .contains("authorization: bearer test-key")
    );
    assert!(sent.contains("\"stream\""));

    assert_eq!(
        events,
        vec![
            StreamEvent::TextDelta("hel".into()),
            StreamEvent::TextDelta("lo".into()),
            StreamEvent::ToolCall(oi_core::runtime::llm::ToolCallSpec {
                id: "c1".into(),
                name: "read_file".into(),
                args: serde_json::json!({"path": "/x"}),
            }),
            StreamEvent::Done {
                stop_reason: StopReason::ToolUse
            },
        ]
    );
}

#[test]
fn http_error_status_yields_error_event() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = sock.read(&mut buf);
        sock.write_all(b"HTTP/1.1 429 Too Many Requests\r\ncontent-length: 9\r\n\r\nrate hit!")
            .unwrap();
    });
    let model = Model {
        api_key: "k".into(),
        model: "m".into(),
        base_url: Some(format!("http://127.0.0.1:{port}/v1")),
        max_tokens: None,
    };
    let events = stream(&model, &Context::default(), &[], &AtomicBool::new(false));
    server.join().unwrap();
    match &events[..] {
        [StreamEvent::Error(e)] => assert!(e.contains("429"), "{e}"),
        other => panic!("expected single error event, got {other:?}"),
    }
}
