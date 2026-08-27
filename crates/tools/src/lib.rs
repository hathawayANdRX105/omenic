//! Built-in tools: read_file / write_file / edit / run_bash.
//!
//! Port of pi-from-scratch `src/tools.ts`. Tools are pure functions over
//! JSON args; they never touch agent state.

use std::io::Read as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

use adaptor::ToolDef;

/// Tool output truncation limit (lines); tail is kept — errors live at the end.
pub const MAX_OUTPUT_LINES: usize = 200;

/// Where full truncated outputs are spilled.
const SPILL_DIR: &str = "/tmp";

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
pub trait Tool {
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

fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::Message(format!("missing string argument: {key}")))
}

/// Truncate to the last `max_lines` lines; full output spills to a temp file.
pub fn truncate_output(content: &str, counter: u64) -> std::io::Result<String> {
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

// ===== read_file =====

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

// ===== write_file =====

pub struct WriteFile;

impl Tool for WriteFile {
    fn name(&self) -> &'static str {
        "write_file"
    }

    fn description(&self) -> String {
        "写入文件（覆盖）。参数：path（路径）、content（内容）".into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"path": {"type": "string"}, "content": {"type": "string"}},
            "required": ["path", "content"],
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let path = arg_str(args, "path")?;
        let content = arg_str(args, "content")?;
        if let Some(parent) = Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(path, content)?;
        Ok(format!("wrote {path} ({} chars)", content.len()))
    }
}

// ===== edit =====

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

// ===== run_bash =====

/// Shell timeout and abort poll interval.
const BASH_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

pub struct RunBash;

/// Run `command` under `sh -c`, killing it after `timeout` or on abort.
/// Returns (exit code if reaped, combined stdout+stderr).
fn run_shell(command: &str, signal: &AtomicBool) -> (Option<i32>, String) {
    let mut child = match Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return (None, format!("[spawn error] {e}")),
    };

    // Drain stdout; stderr merges via the caller's 2>&1 wrapper, so one
    // piped reader suffices and large output can't deadlock the child.
    let mut out = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut out);
    }
    drop(child.stderr.take());

    let deadline = std::time::Instant::now() + BASH_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status.code().unwrap_or(-1)),
            Ok(None) => {
                if signal.load(Ordering::Relaxed) || std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(e) => return (None, format!("{out}[wait error] {e}")),
        }
    };

    let suffix = if status.is_none() {
        if signal.load(Ordering::Relaxed) {
            "\n[aborted]".to_string()
        } else {
            format!("\n[timeout after {}s]", BASH_TIMEOUT.as_secs())
        }
    } else {
        String::new()
    };
    (status, format!("{out}{suffix}"))
}

impl Tool for RunBash {
    fn name(&self) -> &'static str {
        "run_bash"
    }

    fn description(&self) -> String {
        "执行 shell 命令。参数：command（命令字符串）。返回 stdout，截取最后 200 行。超时 30 秒。"
            .into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"command": {"type": "string"}},
            "required": ["command"],
        })
    }

    fn execute(&self, args: &Value, signal: &AtomicBool) -> Result<String, ToolError> {
        let command = arg_str(args, "command")?;
        if signal.load(Ordering::Relaxed) {
            return Ok("aborted".into());
        }
        // Wrap with 2>&1 so stderr interleaves in order without extra pipes.
        let wrapped = format!("({command}) 2>&1");
        let (code, output) = run_shell(&wrapped, signal);
        match code {
            Some(0) => truncate_output(&output, 0).map_err(ToolError::from),
            Some(code) => Ok(truncate_output(&format!("[exit {code}] {output}"), 0)?),
            None => Ok(output),
        }
    }
}

/// The four built-in tools, in registration order.
pub fn builtin_tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(ReadFile),
        Box::new(WriteFile),
        Box::new(EditFile),
        Box::new(RunBash),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn truncation_keeps_tail_and_spills_full_output() {
        let content: String = (0..250).map(|i| format!("line{i}\n")).collect();
        let out = truncate_output(content.trim_end(), 7).unwrap();
        assert!(out.starts_with("[output truncated: showing last 200 of 250 lines."));
        assert!(out.contains("full output: /tmp/oi-output-7.txt"));
        assert!(out.trim_end().ends_with("line249"));
        let full = std::fs::read_to_string("/tmp/oi-output-7.txt").unwrap();
        assert!(full.starts_with("line0\n"));
    }

    #[test]
    fn short_output_not_truncated() {
        assert_eq!(truncate_output("a\nb", 0).unwrap(), "a\nb");
    }

    #[test]
    fn edit_uniqueness_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.txt");
        std::fs::write(&path, "alpha beta alpha").unwrap();
        let sig = sig();

        // No match.
        let err = EditFile
            .execute(
                &json!({"path": path, "old_string": "zzz", "new_string": "x"}),
                &sig,
            )
            .unwrap_err();
        assert!(err.to_string().contains("not found"));

        // Ambiguous.
        let err = EditFile
            .execute(
                &json!({"path": path, "old_string": "alpha", "new_string": "x"}),
                &sig,
            )
            .unwrap_err();
        assert!(err.to_string().contains("matches 2 places"));

        // Unique replace works, including $-literals staying literal.
        EditFile
            .execute(
                &json!({"path": path, "old_string": "beta", "new_string": "$& $1"}),
                &sig,
            )
            .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "alpha $& $1 alpha");
    }

    #[test]
    fn write_creates_parents_and_reports() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b/c.txt");
        let sig = sig();
        let out = WriteFile
            .execute(&json!({"path": path, "content": "hello"}), &sig)
            .unwrap();
        assert_eq!(out, format!("wrote {} (5 chars)", path.display()));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
    }

    #[test]
    fn run_bash_captures_output_and_exit_codes() {
        let sig = sig();
        let ok = RunBash
            .execute(&json!({"command": "echo hi"}), &sig)
            .unwrap();
        assert_eq!(ok.trim(), "hi");

        let fail = RunBash
            .execute(&json!({"command": "echo bad >&2; exit 3"}), &sig)
            .unwrap();
        assert!(fail.contains("[exit 3]"), "got: {fail}");
        assert!(fail.contains("bad"));
    }

    #[test]
    fn run_bash_respects_abort_signal() {
        let sig = AtomicBool::new(true);
        let out = RunBash
            .execute(&json!({"command": "echo nope"}), &sig)
            .unwrap();
        assert_eq!(out, "aborted");
    }

    #[test]
    fn missing_args_rejected() {
        let sig = sig();
        let err = ReadFile.execute(&json!({}), &sig).unwrap_err();
        assert!(err.to_string().contains("missing string argument: path"));
    }

    #[test]
    fn defs_expose_schema_and_names() {
        let tools = builtin_tools();
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert_eq!(names, ["read_file", "write_file", "edit", "run_bash"]);
        for t in &tools {
            let d = def(t.as_ref());
            assert!(!d.description.is_empty());
            assert_eq!(d.parameters["type"], "object");
        }
    }
}
