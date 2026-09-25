use serde_json::{Value, json};
/// JSON-RPC version string constant.
pub const JSONRPC: &str = "2.0";

/// Construct a JSON-RPC request object.
///
/// Creates: {"jsonrpc": "2.0", "id": id, "method": method, "params": params}
pub fn request_value(id: u64, method: &str, params: Value) -> Value {
    json!({
        "jsonrpc": JSONRPC,
        "id": id,
        "method": method,
        "params": params,
    })
}

/// Construct a JSON-RPC notification object.
///
/// Creates: {"jsonrpc": "2.0", "method": method, "params": params}
/// No id field is present in notifications.
pub fn notification_value(method: &str, params: Value) -> Value {
    json!({
        "jsonrpc": JSONRPC,
        "method": method,
        "params": params,
    })
}

/// Encode a JSON-RPC value as a single-line JSON string.
///
/// No trailing newline: [`crate::stdio::write_json_line`] appends it, so
/// serializing and framing stay separate concerns.
pub fn encode_line(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string())
}

/// Decode an NDJSON line into a JSON-RPC value.
///
/// Returns `None` for empty lines, parse errors, or invalid input.
pub fn decode_line(line: &str) -> Option<Value> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    serde_json::from_str(line).ok()
}

/// Classify a JSON-RPC value based on its id and method fields.
///
/// Identifies:
/// - Request: present id + present method
/// - Response: present id + absent method  
/// - Notification: absent id + present method
/// - Ignored: all other cases (including missing id/method, non-numeric id)
pub enum FrameKind<'a> {
    Request {
        id: u64,
        method: &'a str,
        value: &'a Value,
    },
    Response {
        id: u64,
        value: &'a Value,
    },
    Notification {
        method: &'a str,
        value: &'a Value,
    },
    Ignored,
}

/// Classify a JSON-RPC value.
///
/// Uses the same logic as the original ACP `serve` loop:
/// - (Some(id), Some(method)) with numeric id → Request
/// - (Some(id), None) with numeric id → Response  
/// - (None, Some(method)) → Notification
/// - Everything else → Ignored (including non-numeric id)
pub fn classify(value: &Value) -> FrameKind<'_> {
    let id = value.get("id");
    let method = value.get("method").and_then(Value::as_str);

    match (id, method) {
        (Some(id), Some(method)) => {
            if let Some(id_num) = id.as_u64() {
                FrameKind::Request {
                    id: id_num,
                    method,
                    value,
                }
            } else {
                FrameKind::Ignored
            }
        }
        (Some(id), None) => {
            if let Some(id_num) = id.as_u64() {
                FrameKind::Response { id: id_num, value }
            } else {
                FrameKind::Ignored
            }
        }
        (None, Some(method)) => FrameKind::Notification { method, value },
        _ => FrameKind::Ignored,
    }
}
