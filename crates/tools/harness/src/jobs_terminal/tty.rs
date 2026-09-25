//! Persistent-terminal tools (`terminal_*`), moved out of `jobs_terminal.rs`
//! into their own module. Shared helpers stay in the parent, reached via `super::*`.

use super::*;
use serde_json::Value;

use terminal::{TerminalId, TerminalRegistry};
use tools::{Tool, ToolError};

// -----------------------------------------------------------------------------
// terminal_create
// -----------------------------------------------------------------------------

pub struct TerminalCreate {
    reg: Arc<TerminalRegistry>,
}

impl TerminalCreate {
    pub fn new(reg: Arc<TerminalRegistry>) -> Self {
        Self { reg }
    }
}

impl Tool for TerminalCreate {
    fn name(&self) -> &str {
        "terminal_create"
    }

    fn description(&self) -> String {
        "Open a persistent shell session and return its id. Unlike run_bash, state survives \
         between calls: cd, exported variables and running programs persist until the session is \
         closed. Use terminal_write to send commands and terminal_read to see output."
            .into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "shell": {
                    "type": "string",
                    "description": "Shell to launch, with any flags. Defaults to bash."
                },
                "cwd": {
                    "type": "string",
                    "description": "Initial working directory. Defaults to the daemon's cwd."
                },
                "cols": {"type": "integer", "description": "Terminal width. Default 80."},
                "rows": {"type": "integer", "description": "Terminal height. Default 24."}
            }
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let shell = opt_str(args, "shell").unwrap_or_else(|| "bash".to_string());
        let cwd = opt_str(args, "cwd").unwrap_or_else(current_dir);
        let cols = opt_dim(args, "cols", 80)?;
        let rows = opt_dim(args, "rows", 24)?;

        let id = self
            .reg
            .create(&shell, &cwd, cols, rows)
            .map_err(terminal_err)?;
        Ok(format!(
            "terminal {} opened ({shell} in {cwd}, {cols}x{rows})\n\
             Send commands with terminal_write, then read output with terminal_read. \
             The shell echoes input, so output includes the command line itself.",
            id.as_str()
        ))
    }
}

// -----------------------------------------------------------------------------
// terminal_write / terminal_read
// -----------------------------------------------------------------------------

pub struct TerminalWrite {
    reg: Arc<TerminalRegistry>,
}

impl TerminalWrite {
    pub fn new(reg: Arc<TerminalRegistry>) -> Self {
        Self { reg }
    }
}

impl Tool for TerminalWrite {
    fn name(&self) -> &str {
        "terminal_write"
    }

    fn description(&self) -> String {
        "Send input to a terminal session. Include a trailing newline to run a command. The shell \
         echoes what you send, so terminal_read will show your input as well as its output."
            .into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Terminal id from terminal_create."},
                "data": {"type": "string", "description": "Text to send. End with a newline to execute."}
            },
            "required": ["id", "data"]
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let raw = need_str(args, "id")?;
        let data = need_str(args, "data")?;
        let id = TerminalId::new(&raw);
        self.reg.write(&id, &data).map_err(terminal_err)?;
        Ok(format!("wrote {} bytes to terminal {raw}", data.len()))
    }
}

pub struct TerminalRead {
    reg: Arc<TerminalRegistry>,
}

impl TerminalRead {
    pub fn new(reg: Arc<TerminalRegistry>) -> Self {
        Self { reg }
    }
}

impl Tool for TerminalRead {
    fn name(&self) -> &str {
        "terminal_read"
    }

    fn description(&self) -> String {
        "Read output produced since the last read and clear it. Returns promptly even when the \
         shell is idle — it does not wait for output unless you pass timeout_ms. Call it in a loop \
         after terminal_write to collect a command's result."
            .into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Terminal id from terminal_create."},
                "timeout_ms": {
                    "type": "integer",
                    "description": "How long to wait for new output before returning, in milliseconds. Default 0 (return immediately)."
                }
            },
            "required": ["id"]
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let raw = need_str(args, "id")?;
        let timeout = opt_u64(args, "timeout_ms").unwrap_or(0);
        let id = TerminalId::new(&raw);
        let out = self.reg.read(&id, Some(timeout)).map_err(terminal_err)?;

        let text = out.text();
        let (text, complete) = truncate_bytes(&text, READ_CHUNK_BYTES);
        let mut body = if text.is_empty() {
            "(no new output)".to_string()
        } else {
            text
        };
        if !complete {
            body.push_str("\n… (output truncated; call terminal_read again for more)");
        }
        if out.exited {
            body.push_str("\n(terminal exited)");
        }
        Ok(body)
    }
}

// -----------------------------------------------------------------------------
// terminal_resize / terminal_kill / terminal_list
// -----------------------------------------------------------------------------

pub struct TerminalResize {
    reg: Arc<TerminalRegistry>,
}

impl TerminalResize {
    pub fn new(reg: Arc<TerminalRegistry>) -> Self {
        Self { reg }
    }
}

impl Tool for TerminalResize {
    fn name(&self) -> &str {
        "terminal_resize"
    }

    fn description(&self) -> String {
        "Change a terminal's reported width and height. Full-screen programs (editors, pagers) \
         lay themselves out to these dimensions."
            .into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string"},
                "cols": {"type": "integer"},
                "rows": {"type": "integer"}
            },
            "required": ["id", "cols", "rows"]
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let raw = need_str(args, "id")?;
        let cols = need_u16(args, "cols")?;
        let rows = need_u16(args, "rows")?;
        let id = TerminalId::new(&raw);
        self.reg.resize(&id, cols, rows).map_err(terminal_err)?;
        Ok(format!("terminal {raw} resized to {cols}x{rows}"))
    }
}

pub struct TerminalKill {
    reg: Arc<TerminalRegistry>,
}

impl TerminalKill {
    pub fn new(reg: Arc<TerminalRegistry>) -> Self {
        Self { reg }
    }
}

impl Tool for TerminalKill {
    fn name(&self) -> &str {
        "terminal_kill"
    }

    fn description(&self) -> String {
        "Close a terminal session and kill its shell. Any process still running in it is \
         terminated."
            .into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"id": {"type": "string"}},
            "required": ["id"]
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let raw = need_str(args, "id")?;
        let id = TerminalId::new(&raw);
        self.reg.close(&id).map_err(terminal_err)?;
        Ok(format!("closed terminal {raw}"))
    }
}

pub struct TerminalList {
    reg: Arc<TerminalRegistry>,
}

impl TerminalList {
    pub fn new(reg: Arc<TerminalRegistry>) -> Self {
        Self { reg }
    }
}

impl Tool for TerminalList {
    fn name(&self) -> &str {
        "terminal_list"
    }

    fn description(&self) -> String {
        "List open terminal sessions with their shell, working directory and size.".into()
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    fn execute(&self, _args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let rows = self.reg.list();
        if rows.is_empty() {
            return Ok("no terminals".into());
        }
        let mut out = String::new();
        for row in rows {
            let ended = if row.exited { " (exited)" } else { "" };
            out.push_str(&format!(
                "{:<12} {:<24} {:<28} {}x{}{}\n",
                row.id.as_str(),
                row.shell,
                row.cwd,
                row.cols,
                row.rows,
                ended
            ));
        }
        Ok(out.trim_end().to_string())
    }
}
