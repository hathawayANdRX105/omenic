//! JSON-RPC 2.0 framing: value construction, NDJSON round-trips, strict
//! classification, and stdio line framing.

use std::io::BufReader;

use serde_json::json;
use wire::jsonrpc::{
    FrameKind, classify, decode_line, encode_line, notification_value, request_value,
};
use wire::stdio::{read_line, write_json_line};

#[test]
fn request_value_carry_id_method_params() {
    let v = request_value(7, "do", json!({"a": 1}));
    assert_eq!(
        v,
        json!({"jsonrpc": "2.0", "id": 7, "method": "do", "params": {"a": 1}})
    );
}

#[test]
fn notification_value_has_no_id() {
    let v = notification_value("event", json!(null));
    assert_eq!(v.get("id"), None);
    assert_eq!(v["jsonrpc"], "2.0");
    assert_eq!(v["method"], "event");
}

#[test]
fn encode_line_is_single_line_and_classifies_back() {
    let req = request_value(3, "m", json!(1));
    let line = encode_line(&req);
    assert!(!line.contains('\n'));
    let back = decode_line(&line).expect("well-formed line decodes");
    assert!(matches!(
        classify(&back),
        FrameKind::Request { id: 3, method: m, .. } if m == "m"
    ));
}

#[test]
fn classify_response_notification_and_ignored() {
    let resp = json!({"jsonrpc": "2.0", "id": 9, "result": 1});
    assert!(matches!(classify(&resp), FrameKind::Response { id: 9, .. }));

    let note = notification_value("evt", json!(null));
    assert!(matches!(
        classify(&note),
        FrameKind::Notification { method: m, .. } if m == "evt"
    ));
    // Non-object, non-numeric id, and empty object are all Ignored.
    assert!(matches!(classify(&json!([])), FrameKind::Ignored));
    assert!(matches!(
        classify(&json!({"id": "7", "method": "m"})),
        FrameKind::Ignored
    ));
    assert!(matches!(classify(&json!({})), FrameKind::Ignored));
}

#[test]
fn decode_line_rejects_garbage() {
    assert!(decode_line("").is_none());
    assert!(decode_line("   ").is_none());
    assert!(decode_line("{not json").is_none());
}

#[test]
fn write_json_line_appends_newline() {
    let mut out: Vec<u8> = Vec::new();
    write_json_line(&mut out, "abc").unwrap();
    assert_eq!(out.as_slice(), b"abc\n");
}

#[test]
fn read_line_strips_newline_and_reports_eof() {
    let mut reader = BufReader::new(&b"line1\nline2"[..]);
    let mut buf = String::new();

    assert!(read_line(&mut reader, &mut buf).unwrap());
    assert_eq!(buf, "line1");

    assert!(read_line(&mut reader, &mut buf).unwrap());
    assert_eq!(buf, "line2");

    assert!(!read_line(&mut reader, &mut buf).unwrap());
}
