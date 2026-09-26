//! mark_done tool: the model's active completion mark.
//!
//! Zero side effect — it only tells the harness "the task is done". The
//! real stop decision lives in the loop: when the model stops calling tools
//! the host checks whether `mark_done` was called this run (via the tool
//! call event) before letting the turn end. This is the freebuff
//! `task_completed` shape: an explicit-completion gate the run must pass.

use std::sync::atomic::AtomicBool;

use serde_json::Value;

use crate::{Tool, ToolError};

pub struct MarkDone;

impl Tool for MarkDone {
    fn name(&self) -> &'static str {
        "mark_done"
    }

    fn description(&self) -> String {
        "Signal that the assigned task is complete. Call this exactly once, at the very end, only after the deliverable is done and verified. It takes no arguments; the result is an acknowledgement. Do not call it to stop early or to skip remaining work."
            .into()
    }

    fn parameters(&self) -> Value {
        json_obj_empty()
    }

    fn execute(&self, _args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        Ok("marked done".into())
    }
}

/// Empty object schema (no parameters).
fn json_obj_empty() -> Value {
    serde_json::json!({ "type": "object", "properties": {}, "additionalProperties": false })
}
