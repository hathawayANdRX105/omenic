//! apply_patch tool: apply a unified diff (multi-hunk) to one existing file.
//!
//! One patch, one file — the model pre-trains on this format, and a single
//! call can carry several hunks that apply atomically (diffy applies the
//! whole patch or fails without touching the file). Path policy and review
//! gating come from the `Guarded` wrapper, same as `edit`.
use std::sync::atomic::AtomicBool;

use diffy::{Patch, apply};

use serde_json::{Value, json};

use crate::{Tool, ToolError, arg_str};

pub struct ApplyPatch;

impl Tool for ApplyPatch {
    fn name(&self) -> &'static str {
        "apply_patch"
    }

    fn description(&self) -> String {
        "对已存在的文件应用 unified diff 补丁：支持多 hunk 原子应用，任一 hunk 不匹配则整体失败不落盘。参数：path、patch。".into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "patch": {"type": "string"},
            },
            "required": ["path", "patch"],
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let path = arg_str(args, "path")?;
        let patch_text = arg_str(args, "patch")?;

        // diffy parses the whole patch first: a malformed patch never
        // touches the file.
        match Patch::from_str(patch_text) {
            Ok(patch) => {
                // Read the target file
                let content = std::fs::read_to_string(path)
                    .map_err(|e| ToolError::Message(format!("Failed to read {}: {}", path, e)))?;

                // Apply the patch using diffy's apply function
                match apply(&content, &patch) {
                    Ok(new_content) => {
                        std::fs::write(path, &new_content).map_err(|e| {
                            ToolError::Message(format!("Failed to write {}: {}", path, e))
                        })?;
                        Ok(format!("Applied patch to {}", path))
                    }
                    Err(e) => Err(ToolError::Message(format!("Failed to apply patch: {}", e))),
                }
            }
            Err(e) => Err(ToolError::Message(format!("Failed to parse patch: {}", e))),
        }
    }
}
