//! mark_done tool: the model's active completion mark.
//!
//! Zero side effect — it only tells the harness "the task is done". The
//! loop observes the call via `LoopConfig::completion_tool` and ends the
//! run the moment the tool result is recorded (freebuff `task_completed`
//! shape); a *silent* stop without the mark is what the host's bare-
//! continue guard (`LoopConfig::should_continue`) forces back to work.

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
