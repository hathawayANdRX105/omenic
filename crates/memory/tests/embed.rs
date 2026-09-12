//! Embedding backend tests: cosine semantics, the `embedding` field's serde
//! compatibility, and the OpenAI-compatible HTTP client against a local mock
//! server (scripted raw responses, same pattern as `adaptor/tests/retry.rs`).

use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use memory::{EmbedError, Embedder, MemoryEntry, OpenAIEmbeddings, cosine};

// ===== cosine =====

#[test]
fn cosine_orthogonal_same_and_opposite() {
    assert_eq!(cosine(&[1.0, 0.0], &[0.0, 1.0]), Some(0.0), "orthogonal");
    assert_eq!(
        cosine(&[1.0, 0.0], &[2.0, 0.0]),
        Some(1.0),
        "same direction"
    );
    assert_eq!(cosine(&[1.0, 0.0], &[-3.0, 0.0]), Some(-1.0), "opposite");
    // Unnormalized but parallel vectors still score 1.
    assert_eq!(cosine(&[1.0, 2.0], &[1.0, 2.0]), Some(1.0));
}

#[test]
fn cosine_none_on_length_mismatch_or_zero_vector() {
    assert_eq!(
        cosine(&[1.0, 0.0], &[1.0, 0.0, 0.0]),
        None,
        "length differs"
    );
    assert_eq!(cosine(&[0.0, 0.0], &[1.0, 0.0]), None, "zero query");
    assert_eq!(cosine(&[1.0, 0.0], &[0.0, 0.0]), None, "zero candidate");
    assert_eq!(cosine(&[0.0], &[0.0]), None, "both zero");
}

// ===== embedding field serde =====

#[test]
fn embedding_field_round_trips() {
    let mut entry = MemoryEntry::new("user prefers tabs");
    entry.embedding = Some(vec![0.25, -0.5, 1.0]);

    let line = serde_json::to_string(&entry).unwrap();
    assert!(line.contains("\"embedding\":[0.25,-0.5,1.0]"), "{line}");
    let back: MemoryEntry = serde_json::from_str(&line).unwrap();
    assert_eq!(back, entry);
}

#[test]
fn old_store_shape_without_embedding_loads() {
    let entry: MemoryEntry =
        serde_json::from_str("{\"id\":1,\"ts\":\"t\",\"text\":\"ok\"}").unwrap();
    assert_eq!(
        entry.embedding, None,
        "old rows must load with no embedding"
    );
    assert!(entry.active, "old rows keep the other serde defaults too");
}

// ===== mock HTTP server =====

/// A server that answers each connection with scripted raw responses, counts
/// connections, and captures the first request bytes (same pattern as
/// `adaptor/tests/retry.rs`).
fn serve_scripted(responses: Vec<String>) -> (u16, Arc<AtomicUsize>, Arc<Mutex<Vec<u8>>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let served = Arc::new(AtomicUsize::new(0));
    let request = Arc::new(Mutex::new(Vec::new()));
    let served_clone = Arc::clone(&served);
    let request_clone = Arc::clone(&request);
    std::thread::spawn(move || {
        for resp in responses {
            let Ok((mut sock, _)) = listener.accept() else {
                break;
            };
            served_clone.fetch_add(1, Ordering::SeqCst);

            // Read until the request is complete: headers plus `Content-Length`
            // body bytes. Unlike the adaptor tests (which never inspect the
            // request), one read() can return the header segment before the
            // body segment lands, so keep reading with a short backstop.
            let _ = sock.set_read_timeout(Some(std::time::Duration::from_millis(10)));
            let mut captured: Vec<u8> = Vec::new();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while !request_complete(&captured) && std::time::Instant::now() < deadline {
                let mut buf = [0u8; 4096];
                match sock.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => captured.extend_from_slice(&buf[..n]),
                    // Read timeout: park briefly instead of busy-spinning
                    // through the 2s deadline on a stalled body.
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(2)),
                }
            }
            request_clone.lock().unwrap().extend_from_slice(&captured);

            if sock.write_all(resp.as_bytes()).is_err() {
                break;
            }
            let _ = sock.flush();
            // Dropping a socket that still holds unread request data sends
            // RST and can clobber the response; drain briefly instead.
            let _ = sock.set_read_timeout(Some(std::time::Duration::from_millis(50)));
            let mut drain = [0u8; 4096];
            while let Ok(n) = sock.read(&mut drain) {
                if n == 0 {
                    break;
                }
            }
        }
    });
    (port, served, request)
}

/// Whether `captured` holds a complete HTTP request: a header block plus the
/// body length its `Content-Length` announces.
fn request_complete(captured: &[u8]) -> bool {
    let Some(header_end) = captured
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| p + 4)
    else {
        return false;
    };
    let headers = String::from_utf8_lossy(&captured[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|l| {
            let (name, value) = l.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    captured.len() >= header_end + content_length
}

fn json_ok(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn status(status: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn client(port: u16) -> OpenAIEmbeddings {
    OpenAIEmbeddings::new(
        format!("http://127.0.0.1:{port}/v1"),
        "test-key",
        "text-embedding-3-small",
    )
}

fn texts<const N: usize>(items: [&str; N]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn embeddings_200_returns_vectors_in_request_order() {
    let (port, served, request) = serve_scripted(vec![json_ok(
        r#"{"data":[
              {"index":0,"embedding":[1.0,0.0]},
              {"index":1,"embedding":[0.0,1.0]}]}"#,
    )]);
    let got = client(port).embed(&texts(["a", "b"])).unwrap();
    assert_eq!(got, vec![vec![1.0, 0.0], vec![0.0, 1.0]]);
    assert_eq!(served.load(Ordering::SeqCst), 1);

    // `input` must be sent as an array of strings (some OpenAI-compatible
    // implementations reject the bare-string form), with auth and model.
    let req = String::from_utf8(request.lock().unwrap().clone()).unwrap();
    assert!(req.contains("POST /v1/embeddings"), "{req}");
    assert!(req.contains("Bearer test-key"), "{req}");
    assert!(
        req.contains("\"model\":\"text-embedding-3-small\""),
        "{req}"
    );
    assert!(req.contains("\"input\":[\"a\",\"b\"]"), "{req}");
}

#[test]
fn embeddings_out_of_order_data_restored_by_index() {
    let (port, _served, _request) = serve_scripted(vec![json_ok(
        r#"{"data":[
              {"index":2,"embedding":[3.0,3.0]},
              {"index":0,"embedding":[1.0,1.0]},
              {"index":1,"embedding":[2.0,2.0]}]}"#,
    )]);
    let got = client(port).embed(&texts(["x", "y", "z"])).unwrap();
    assert_eq!(
        got,
        vec![vec![1.0, 1.0], vec![2.0, 2.0], vec![3.0, 3.0]],
        "vectors must come back in request order, not response order"
    );
}

#[test]
fn embeddings_missing_index_falls_back_to_array_position() {
    let (port, _served, _request) = serve_scripted(vec![json_ok(
        r#"{"data":[{"embedding":[7.0]},{"embedding":[8.0]}]}"#,
    )]);
    let got = client(port).embed(&texts(["a", "b"])).unwrap();
    assert_eq!(got, vec![vec![7.0], vec![8.0]]);
}

#[test]
fn embeddings_non_200_is_a_status_error() {
    let (port, served, _request) =
        serve_scripted(vec![status("401 Unauthorized", r#"{"error":"bad key"}"#)]);
    let err = client(port).embed(&texts(["a"])).unwrap_err();
    match err {
        EmbedError::Status { status, body } => {
            assert_eq!(status, 401);
            assert!(body.contains("bad key"), "{body}");
        }
        other => panic!("expected Status, got {other:?}"),
    }
    assert_eq!(served.load(Ordering::SeqCst), 1, "must not retry");
}

#[test]
fn embeddings_wrong_vector_count_is_an_error() {
    let (port, _served, _request) =
        serve_scripted(vec![json_ok(r#"{"data":[{"index":0,"embedding":[1.0]}]}"#)]);
    let err = client(port).embed(&texts(["a", "b"])).unwrap_err();
    assert!(
        matches!(err, EmbedError::Count { want: 2, got: 1 },),
        "{err:?}"
    );
}

#[test]
fn embeddings_missing_data_array_is_a_parse_error() {
    let (port, _served, _request) = serve_scripted(vec![json_ok(r#"{"oops":true}"#)]);
    let err = client(port).embed(&texts(["a"])).unwrap_err();
    assert!(matches!(err, EmbedError::Parse(_)), "{err:?}");
}

#[test]
fn empty_input_makes_no_http_call() {
    let (port, served, _request) = serve_scripted(vec![]);
    let got = client(port).embed(&[]).unwrap();
    assert!(got.is_empty());
    assert_eq!(served.load(Ordering::SeqCst), 0, "no request for no input");
}
