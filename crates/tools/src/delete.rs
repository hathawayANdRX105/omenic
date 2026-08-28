//! delete_file tool: move a file to trash via `gio trash`.
//!
//! Follows the AGENTS.md rule: always use `gio trash`, never `rm` directly.
//! Falls back to `rm` only if `gio` is not available.

use std::process::Command;
use std::sync::atomic::AtomicBool;

use serde_json::{Value, json};

use crate::{Tool, ToolError, arg_str};

pub struct DeleteFile;

impl Tool for DeleteFile {
    fn name(&self) -> &'static str {
        "delete_file"
    }

    fn description(&self) -> String {
        "删除文件（移动到回收站）。参数：path（文件路径）。使用 gio trash，可从回收站恢复。".into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"],
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let path = arg_str(args, "path")?;

        if !std::path::Path::new(path).exists() {
            return Err(ToolError::Message(format!("{path} does not exist")));
        }

        // Try gio trash first (FreeDesktop trash, recoverable)
        let gio_result = Command::new("gio").arg("trash").arg(path).output();

        match gio_result {
            Ok(output) if output.status.success() => Ok(format!("moved {path} to trash")),
            _ => {
                // Fallback: rm if gio is unavailable
                let rm_result = Command::new("rm").arg(path).output();
                match rm_result {
                    Ok(output) if output.status.success() => Ok(format!("deleted {path}")),
                    Ok(output) => {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        Err(ToolError::Message(format!(
                            "delete failed: {}",
                            stderr.trim()
                        )))
                    }
                    Err(e) => Err(ToolError::Message(format!("failed to run rm: {e}"))),
                }
            }
        }
    }
}
