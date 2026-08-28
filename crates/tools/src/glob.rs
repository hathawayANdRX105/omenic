//! glob tool: list files matching a glob pattern using `rg --files`.
//!
//! Runs `rg --files --glob=<pattern> --sort=modified` as a subprocess,
//! returns matching paths. Respects gitignore by default.

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::{Tool, ToolError, arg_str, truncate_output};

const RG_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

pub struct Glob;

impl Tool for Glob {
    fn name(&self) -> &'static str {
        "glob"
    }

    fn description(&self) -> String {
        "按 glob pattern 列文件路径。参数：pattern（glob，如 **/*.rs）、path（搜索目录，默认当前目录）。".into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string"},
                "path": {"type": "string"},
            },
            "required": ["pattern"],
        })
    }

    fn execute(&self, args: &Value, signal: &AtomicBool) -> Result<String, ToolError> {
        let pattern = arg_str(args, "pattern")?;
        let path = args.get("path").and_then(Value::as_str).unwrap_or(".");

        let mut cmd = Command::new("rg");
        cmd.arg("--files")
            .arg("--sort=modified")
            .arg(format!("--glob={pattern}"))
            .arg("--")
            .arg(path);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| ToolError::Message(format!("failed to spawn rg: {e}")))?;

        let mut output = child.stdout.take().unwrap();
        let deadline = Instant::now() + RG_TIMEOUT;

        let reader = std::thread::spawn(move || {
            use std::io::Read;
            let mut buf = Vec::new();
            output.read_to_end(&mut buf).unwrap_or(0);
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
                return Err(ToolError::Message("rg timed out".into()));
            }
            std::thread::sleep(POLL_INTERVAL);
        }

        let raw = reader.join().unwrap_or_default();
        let stdout = String::from_utf8_lossy(&raw);

        let paths: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
        if paths.is_empty() {
            return Ok("no files matched".into());
        }

        let header = format!("{} files:\n", paths.len());
        let body = paths.join("\n");
        Ok(truncate_output(&format!("{header}{body}"), 0)?)
    }
}
