//! Subcommand handler modules, split out of `cli.rs` by concern.
//!
//! `print_json` / `json_ok` are shared by every handler and live here so the
//! family modules reach them via `super::*`.

mod render;
mod sess;
mod specfns;
mod sub;
mod task;
mod workspace;

pub fn print_json<T: ?Sized + serde::Serialize>(value: &T) {
    println!(
        "{}",
        serde_json::to_string_pretty(value)
            .unwrap_or_else(|e| { format!("{{\"error\":\"json serialization failed: {e}\"}}") })
    );
}

/// Print `{"status":"ok","message":"..."}` when --json is set.
pub fn json_ok(message: &str) {
    let obj = serde_json::json!({"status": "ok", "message": message});
    print_json(&obj);
}

pub use render::*;
pub use sess::*;
pub use specfns::*;
pub use sub::*;
pub use task::*;
pub use workspace::*;
