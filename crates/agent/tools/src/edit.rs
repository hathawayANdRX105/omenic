//! edit tool: precise text replacement with uniqueness enforcement.

use std::sync::atomic::AtomicBool;

use serde_json::{Value, json};

use crate::{Tool, ToolError, arg_str};

pub struct EditFile;

impl Tool for EditFile {
    fn name(&self) -> &'static str {
        "edit"
    }

    fn description(&self) -> String {
        "编辑文件：精确替换一段文本。参数：path、old_string、new_string。old_string 必须在文件中唯一匹配，否则报错。".into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "old_string": {"type": "string"},
                "new_string": {"type": "string"},
            },
            "required": ["path", "old_string", "new_string"],
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let path = arg_str(args, "path")?;
        let old = arg_str(args, "old_string")?;
        let new = arg_str(args, "new_string")?;
        if old.is_empty() {
            return Err(ToolError::Message("old_string must not be empty".into()));
        }
        let content = std::fs::read_to_string(path)?;
        let count = content.matches(old).count();
        if count == 0 {
            return Err(ToolError::Message(format!(
                "old_string not found in {path}"
            )));
        }
        if count > 1 {
            return Err(ToolError::Message(format!(
                "old_string matches {count} places in {path}, must be unique"
            )));
        }
        std::fs::write(path, content.replacen(old, new, 1))?;
        Ok(format!("edited {path}: replaced {} chars", old.len()))
    }
}
