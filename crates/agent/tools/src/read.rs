//! read_file tool: read file content, truncate large output.

use std::sync::atomic::AtomicBool;

use serde_json::{Value, json};

use crate::{Tool, ToolError, arg_str, truncate_output};

pub struct ReadFile;

impl Tool for ReadFile {
    fn name(&self) -> &'static str {
        "read_file"
    }

    fn description(&self) -> String {
        "读取文件内容。参数：path（文件路径）。大文件截取最后 200 行。".into()
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]})
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let path = arg_str(args, "path")?;
        let content = std::fs::read_to_string(path)?;
        Ok(truncate_output(&content, 0)?)
    }
}
