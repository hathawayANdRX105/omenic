//! str_replace_editor tool: Morph-style string-replace editor with uniqueness requirement.
use std::sync::atomic::AtomicBool;

use serde_json::{Value, json};

use crate::{Tool, ToolError, arg_str};

pub struct StrReplaceEditor;

impl Tool for StrReplaceEditor {
    fn name(&self) -> &'static str {
        "str_replace_editor"
    }

    fn description(&self) -> String {
        "字符串替换编辑器：支持精确替换、行范围替换、查看文件/目录。old_str 必须唯一匹配。".into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "enum": ["str_replace", "view", "create"]},
                "path": {"type": "string"},
                "old_str": {"type": "string"},
                "new_str": {"type": "string"},
                "view_range": {
                    "type": "array",
                    "items": {"type": "integer"},
                    "minItems": 2,
                    "maxItems": 2
                },
            },
            "required": ["command", "path"],
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let command = arg_str(args, "command")?;
        let path = arg_str(args, "path")?;

        match command {
            "view" => self.view_file(path, args),
            "str_replace" => self.str_replace(path, args),
            "create" => self.create_file(path, args),
            _ => Err(ToolError::Message(format!("Unknown command: {}", command))),
        }
    }
}

impl StrReplaceEditor {
    fn view_file(&self, path: &str, args: &Value) -> Result<String, ToolError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| ToolError::Message(format!("Failed to read {}: {}", path, e)))?;
        if let Some(view_range) = args.get("view_range").and_then(|v| v.as_array()) {
            let start_1based = view_range[0].as_i64().unwrap_or(1) as usize;
            let end_1based = view_range[1]
                .as_i64()
                .unwrap_or(content.lines().count() as i64) as usize;

            let lines: Vec<&str> = content.lines().collect();
            // Convert to 0-based, with bounds checking
            let start = start_1based.saturating_sub(1).min(lines.len());
            let end = end_1based.min(lines.len());

            let result: Vec<String> = lines[start..end]
                .iter()
                .enumerate()
                .map(|(i, l)| format!("{:4} {}", start + i + 1, l))
                .collect();

            Ok(result.join("\n"))
        } else {
            // Show first 100 lines
            let lines: Vec<&str> = content.lines().collect();
            let limit = lines.len().min(100);
            let result: Vec<String> = lines[..limit]
                .iter()
                .enumerate()
                .map(|(i, l)| format!("{:4} {}", i + 1, l))
                .collect();
            Ok(result.join("\n"))
        }
    }

    fn str_replace(&self, path: &str, args: &Value) -> Result<String, ToolError> {
        let old_str = arg_str(args, "old_str")?;
        let new_str = arg_str(args, "new_str")?;

        if old_str.is_empty() {
            return Err(ToolError::Message("old_string must not be empty".into()));
        }

        let content = std::fs::read_to_string(path)
            .map_err(|e| ToolError::Message(format!("Failed to read {}: {}", path, e)))?;

        let count = content.matches(old_str).count();
        if count == 0 {
            return Err(ToolError::Message(format!(
                "old_string not found in {}",
                path
            )));
        }
        if count > 1 {
            return Err(ToolError::Message(format!(
                "old_string matches {} places in {}, must be unique",
                count, path
            )));
        }

        let new_content = content.replacen(old_str, new_str, 1);
        std::fs::write(path, &new_content)
            .map_err(|e| ToolError::Message(format!("Failed to write {}: {}", path, e)))?;

        Ok(format!("edited {}: replaced {} chars", path, old_str.len()))
    }

    fn create_file(&self, path: &str, args: &Value) -> Result<String, ToolError> {
        let new_str = arg_str(args, "new_str")?;

        if std::path::Path::new(path).exists() {
            return Err(ToolError::Message(format!("File already exists: {}", path)));
        }

        // Create parent directories if needed
        if let Some(parent) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ToolError::Message(format!("Failed to create directories: {}", e)))?;
        }

        std::fs::write(path, new_str)
            .map_err(|e| ToolError::Message(format!("Failed to write {}: {}", path, e)))?;

        Ok(format!("created {}", path))
    }
}
