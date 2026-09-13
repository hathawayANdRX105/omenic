//! run_bash tool: execute shell commands with timeout and abort.

use std::io::Read as _;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

use crate::{Tool, ToolError, arg_str, truncate_output};

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
