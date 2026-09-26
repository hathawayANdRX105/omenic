//! Attachments survive the RPC boundary: a `worker.prompt` carrying an image
//! must reach the engine as blocks, and a malformed attachment must be
//! refused before it can reach a provider.

use serde_json::{Value, json};

use daemon::protocol::{Command, Request};

/// Prompt params with the base fields every caller sends, plus extras.
fn params(extra: &Value) -> Value {
    let mut map = json!({ "message": "look at this", "session_id": "s-1", "run_id": "r-1" });
    if let (Some(dst), Some(src)) = (map.as_object_mut(), extra.as_object()) {
        for (k, v) in src {
            dst.insert(k.clone(), v.clone());
        }
    }
    map
}

fn png(data: &str) -> Value {
    json!({ "name": "shot.png", "media_type": "image/png", "data": data })
}

#[test]
fn no_attachments_field_means_none() {
    let parsed = daemon::state::optional_attachments(&params(&json!({}))).unwrap();
    assert!(parsed.is_empty());
}

#[test]
fn empty_attachments_array_means_none() {
    let parsed =
        daemon::state::optional_attachments(&params(&json!({ "attachments": [] }))).unwrap();
    assert!(parsed.is_empty());
}

#[test]
fn valid_png_attachment_parses() {
    let p = params(&json!({ "attachments": [png("aGVsbG8=")] }));
    let parsed = daemon::state::optional_attachments(&p).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].name, "shot.png");
    assert_eq!(parsed[0].media_type, "image/png");
    assert_eq!(parsed[0].data, "aGVsbG8=");
}

#[test]
fn name_is_optional() {
    let p = params(&json!({
        "attachments": [{ "media_type": "image/png", "data": "aGk=" }]
    }));
    let parsed = daemon::state::optional_attachments(&p).unwrap();
    assert_eq!(parsed[0].name, "");
}

#[test]
fn unsupported_media_type_is_refused() {
    let p = params(&json!({
        "attachments": [{ "media_type": "application/pdf", "data": "aGk=" }]
    }));
    let err = daemon::state::optional_attachments(&p).unwrap_err();
    assert!(err.contains("unsupported attachment media type"), "{err}");
}

#[test]
fn data_url_prefixed_payload_is_refused() {
    // A `data:`-prefixed or otherwise non-base64 payload must not pass: it
    // would be concatenated straight into the provider's data: URL.
    let p = params(&json!({ "attachments": [png("data:image/png;base64,aGk=")] }));
    let err = daemon::state::optional_attachments(&p).unwrap_err();
    assert!(err.contains("not valid base64"), "{err}");
}

#[test]
fn empty_payload_is_refused() {
    let p = params(&json!({ "attachments": [png("")] }));
    let err = daemon::state::optional_attachments(&p).unwrap_err();
    assert!(err.contains("empty"), "{err}");
}

#[test]
fn too_many_attachments_is_refused() {
    let many: Vec<Value> = (0..5).map(|_| png("aGk=")).collect();
    let p = params(&json!({ "attachments": many }));
    let err = daemon::state::optional_attachments(&p).unwrap_err();
    assert!(err.contains("at most 4 attachments"), "{err}");
}

#[test]
fn non_array_attachments_is_refused() {
    let p = params(&json!({ "attachments": "aGk=" }));
    let err = daemon::state::optional_attachments(&p).unwrap_err();
    assert!(err.contains("must be an array"), "{err}");
}

#[test]
fn prompt_request_round_trips_attachments() {
    let req = Request {
        id: Some("7".into()),
        command: Command::WorkerPrompt,
        params: params(&json!({ "attachments": [png("aGk=")] })),
    };
    let back: Request = serde_json::from_value(serde_json::to_value(&req).unwrap()).unwrap();
    assert_eq!(back.command, Command::WorkerPrompt);
    let parsed = daemon::state::optional_attachments(&back.params).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].media_type, "image/png");
}
