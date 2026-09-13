//! The single daemon-backed tool exposed to omp.
//!
//! `session_query` is the only tool the daemon registers with the worker via
//! [`rpc::worker::Worker::register_external_tools`]. Its execution lives
//! behind a future tool-call dispatcher; for now the daemon registers the
//! `ToolDef` so the agent's schema reflects a daemon-backed entry point,
//! and [`Client::session_query`] on the daemon client implements the same
//! payload shape end-to-side.
//!
//! ponytail: we don't synthesize a full executor here because the daemon
//! owns the `SessionDb` already — the agent's tool result would just be
//! `Client::session_query(args)`. That's how CLI/TUI side paths stay in
//! step with the agent-facing surface.

use adaptor::ToolDef;
use serde_json::{Value, json};

/// Canonical tool name as registered with the worker.
pub const SESSION_QUERY_NAME: &str = "session_query";

/// The `ToolDef` the daemon registers on every worker spawn. One def,
/// one name — exactly one entry in the agent's tool list from this side.
pub fn session_query_def() -> ToolDef {
    ToolDef {
        name: SESSION_QUERY_NAME.to_string(),
        description: "Query the daemon-backed session database. Args: \
                      `{ \"kind\": \"list\"|\"get\"|\"search\"|\"delete\", \
                        \"query\"?: string, \"session_id\"?: string, \
                        \"scope_id\"?: string, \"limit\"?: number }`. \
                      Returns JSON: list/get/search yield sessions or \
                      messages; delete yields `{ \"deleted\": bool }`."
            .to_string(),
        parameters: json!({
            "type": "object",
            "required": ["kind"],
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": ["list", "get", "search", "delete"],
                },
                "query": { "type": "string" },
                "session_id": { "type": "string" },
                "scope_id": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 1000 },
            },
        }),
    }
}

/// Sanity helper: parse an args blob and return the canonical
/// `kind` value or an error message. Keeps call sites boring and
/// lets tests assert shape without re-implementing the JSON parse.
#[allow(dead_code)] // used by future tool executor wiring
pub fn kind_of(args: &Value) -> Option<&str> {
    args.get("kind").and_then(Value::as_str)
}
