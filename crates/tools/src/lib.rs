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

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use adaptor::ToolDef;
use memory::{Memory, MemoryEntry, MemoryError};
use serde_json::{Value, json};

/// Tool output truncation limit (lines); tail is kept — errors live at the end.
pub const MAX_OUTPUT_LINES: usize = 200;

/// Where full truncated outputs are spilled.
pub const SPILL_DIR: &str = "/tmp";

/// Subprocess timeout and abort poll interval shared by rg-based tools.
pub const RG_TIMEOUT: Duration = Duration::from_secs(30);
pub const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Run a `Command` as a subprocess with timeout + abort support.
/// Spawns the command, reads stdout in a background thread (avoids
/// deadlock when the child's stderr pipe fills), and polls for
/// completion / abort / timeout.
///
/// Returns the complete stdout bytes on success, or a `ToolError` on
/// abort / timeout / spawn failure.
pub fn run_subprocess(
    mut cmd: std::process::Command,
    signal: &AtomicBool,
) -> Result<Vec<u8>, ToolError> {
    use std::process::Stdio;
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| ToolError::Message(format!("failed to spawn subprocess: {e}")))?;

    let mut stdout = child.stdout.take().unwrap();
    let deadline = Instant::now() + RG_TIMEOUT;

    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = Vec::new();
        stdout.read_to_end(&mut buf).unwrap_or(0);
        buf
    });

    loop {
        if signal.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ToolError::Message("aborted".into()));
        }
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(_) => break,
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ToolError::Message("subprocess timed out".into()));
        }
        std::thread::sleep(POLL_INTERVAL);
    }

    Ok(reader.join().unwrap_or_default())
}

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

/// Memory-backed tools: `memory_append` / `memory_list` / `memory_search`.
///
/// Registered separately from [`builtin_tools`] so the default tool set stays
/// unchanged. A [`Memory::disabled`] handle still yields all three tools —
/// they answer with a clear "memory disabled" error instead of vanishing, so
/// the model learns why the call failed rather than hallucinating the tool.
pub fn external_tools_from_memory(memory: Memory) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(MemoryAppend {
            memory: memory.clone(),
        }),
        Box::new(MemoryList {
            memory: memory.clone(),
        }),
        Box::new(MemorySearch { memory }),
    ]
}

const MEMORY_DISABLED: &str =
    "memory disabled: set `enabled = true` under `[memory]` in .oi/config.toml to use it";

fn check_enabled(memory: &Memory) -> Result<(), ToolError> {
    if memory.enabled() {
        Ok(())
    } else {
        Err(ToolError::Message(MEMORY_DISABLED.into()))
    }
}

fn render(entries: &[MemoryEntry]) -> Result<String, ToolError> {
    if entries.is_empty() {
        return Ok("no memory entries".into());
    }
    let body: Vec<String> = entries
        .iter()
        .map(|e| format!("#{} {} {}", e.id, e.ts, e.text))
        .collect();
    Ok(truncate_output(&body.join("\n"), 0)?)
}

pub struct MemoryAppend {
    memory: Memory,
}

impl Tool for MemoryAppend {
    fn name(&self) -> &'static str {
        "memory_append"
    }

    fn description(&self) -> String {
        "记一条长期记忆。参数：text（要记住的内容）。".into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"text": {"type": "string"}},
            "required": ["text"],
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        check_enabled(&self.memory)?;
        let text = arg_str(args, "text")?;
        if text.trim().is_empty() {
            return Err(ToolError::Message("text must not be empty".into()));
        }
        // ponytail: `Memory` is just a path — id assignment and the write happen
        // under its own file lock, so a clone is a handle, not a second copy of
        // state. Cheaper than wrapping every tool in a Mutex that guards nothing.
        let mut memory = self.memory.clone();
        memory.append(MemoryEntry::new(text))?;
        Ok("remembered".into())
    }
}

pub struct MemoryList {
    memory: Memory,
}

impl Tool for MemoryList {
    fn name(&self) -> &'static str {
        "memory_list"
    }

    fn description(&self) -> String {
        "列出全部长期记忆（按 id 升序）。无参数。".into()
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    fn execute(&self, _args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        check_enabled(&self.memory)?;
        render(&self.memory.list()?)
    }
}

pub struct MemorySearch {
    memory: Memory,
}

impl Tool for MemorySearch {
    fn name(&self) -> &'static str {
        "memory_search"
    }

    fn description(&self) -> String {
        "在长期记忆里按子串搜索（忽略大小写）。参数：query（子串，非正则）。".into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"query": {"type": "string"}},
            "required": ["query"],
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        check_enabled(&self.memory)?;
        let query = arg_str(args, "query")?;
        render(&self.memory.search(query)?)
    }
}

impl From<MemoryError> for ToolError {
    fn from(e: MemoryError) -> ToolError {
        ToolError::Message(format!("memory: {e}"))
    }
}

#[cfg(test)]
mod memory_tool_tests {
    use super::*;

    fn run(tools: &[Box<dyn Tool>], name: &str, args: Value) -> Result<String, ToolError> {
        let tool = tools
            .iter()
            .find(|t| t.name() == name)
            .unwrap_or_else(|| panic!("tool {name} not registered"));
        tool.execute(&args, &AtomicBool::new(false))
    }

    #[test]
    fn builtin_tools_have_no_memory_tools() {
        let names: Vec<&str> = builtin_tools().iter().map(|t| t.name()).collect();
        assert_eq!(names.len(), 7);
        assert!(!names.iter().any(|n| n.starts_with("memory_")));
    }

    #[test]
    fn disabled_memory_still_registers_and_explains() {
        let tools = external_tools_from_memory(Memory::disabled());
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert_eq!(names, vec!["memory_append", "memory_list", "memory_search"]);

        for (name, args) in [
            ("memory_append", json!({"text": "x"})),
            ("memory_list", json!({})),
            ("memory_search", json!({"query": "x"})),
        ] {
            let err = run(&tools, name, args).expect_err("disabled memory must error");
            assert!(
                err.to_string().contains("memory disabled"),
                "{name}: {err}"
            );
        }
    }

    #[test]
    fn enabled_memory_append_list_search() {
        let tmp = tempfile::tempdir().unwrap();
        let tools = external_tools_from_memory(Memory::open(tmp.path()).unwrap());

        assert_eq!(run(&tools, "memory_list", json!({})).unwrap(), "no memory entries");
        run(&tools, "memory_append", json!({"text": "prefers ripgrep"})).unwrap();
        run(&tools, "memory_append", json!({"text": "deploys on fly.io"})).unwrap();

        let listed = run(&tools, "memory_list", json!({})).unwrap();
        assert!(listed.contains("#1 "), "{listed}");
        assert!(listed.contains("prefers ripgrep"), "{listed}");
        assert!(listed.contains("deploys on fly.io"), "{listed}");

        let found = run(&tools, "memory_search", json!({"query": "RIPGREP"})).unwrap();
        assert!(found.contains("prefers ripgrep"), "{found}");
        assert!(!found.contains("fly.io"), "{found}");
        assert_eq!(
            run(&tools, "memory_search", json!({"query": "nope"})).unwrap(),
            "no memory entries"
        );
    }

    #[test]
    fn append_rejects_blank_text() {
        let tmp = tempfile::tempdir().unwrap();
        let tools = external_tools_from_memory(Memory::open(tmp.path()).unwrap());
        let err = run(&tools, "memory_append", json!({"text": "   "})).expect_err("blank");
        assert!(err.to_string().contains("must not be empty"), "{err}");
        let err = run(&tools, "memory_append", json!({})).expect_err("missing arg");
        assert!(err.to_string().contains("text"), "{err}");
    }
}
