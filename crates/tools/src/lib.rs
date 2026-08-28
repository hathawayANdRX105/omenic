//! Built-in tools: read / write / edit / bash.
//!
//! Shared types and registration. Each tool lives in its own file and
//! registers itself via `register()`.

pub mod bash;
pub mod delete;
pub mod edit;
pub mod glob;
pub mod grep;
pub mod read;
pub mod write;

use std::sync::atomic::AtomicBool;

use adaptor::ToolDef;
use serde_json::Value;

/// Tool output truncation limit (lines); tail is kept — errors live at the end.
pub const MAX_OUTPUT_LINES: usize = 200;

/// Where full truncated outputs are spilled.
pub const SPILL_DIR: &str = "/tmp";

/// Errors returned by tool execution.
#[derive(Debug)]
pub enum ToolError {
    Io(std::io::Error),
    Message(String),
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolError::Io(e) => write!(f, "{e}"),
            ToolError::Message(m) => write!(f, "{m}"),
        }
    }
}

impl From<std::io::Error> for ToolError {
    fn from(e: std::io::Error) -> Self {
        ToolError::Io(e)
    }
}

/// A tool the agent can call. `execute` returns its result as a string
/// (errors are values — the loop backfills them into the context).
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> String;
    fn parameters(&self) -> Value;
    fn execute(&self, args: &Value, signal: &AtomicBool) -> Result<String, ToolError>;
}

/// API-facing definition for any tool.
pub fn def(tool: &dyn Tool) -> ToolDef {
    ToolDef {
        name: tool.name().to_string(),
        description: tool.description(),
        parameters: tool.parameters(),
    }
}

/// Extract a string argument from JSON args, or error.
pub fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::Message(format!("missing string argument: {key}")))
}

/// Truncate to the last `MAX_OUTPUT_LINES` lines; full output spills to a temp file.
pub fn truncate_output(content: &str, counter: u64) -> std::io::Result<String> {
    use std::path::Path;

    let line_count = content.lines().count();
    if line_count <= MAX_OUTPUT_LINES {
        return Ok(content.to_string());
    }
    let kept: Vec<&str> = content
        .lines()
        .skip(line_count - MAX_OUTPUT_LINES)
        .collect();
    let spill_path = Path::new(SPILL_DIR).join(format!("oi-output-{counter}.txt"));
    std::fs::write(&spill_path, content)?;
    Ok(format!(
        "[output truncated: showing last {MAX_OUTPUT_LINES} of {line_count} lines. full output: {}]\n{}",
        spill_path.display(),
        kept.join("\n")
    ))
}

/// Register all built-in tools.
pub fn builtin_tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(read::ReadFile),
        Box::new(write::WriteFile),
        Box::new(edit::EditFile),
        Box::new(bash::RunBash),
        Box::new(grep::Grep),
        Box::new(glob::Glob),
        Box::new(delete::DeleteFile),
    ]
}
