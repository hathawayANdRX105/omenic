//! grep tool: search file contents using ripgrep (`rg --json`).
//!
//! Runs `rg` as a subprocess with `--json` output, parses NDJSON match
//! records, and returns `path:line: content` lines. Respects gitignore by
//! default; abort is polled via the signal flag.

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::{Tool, ToolError, arg_str, truncate_output};

const RG_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const MAX_MATCHES: usize = 200;

pub struct Grep;

impl Tool for Grep {
    fn name(&self) -> &'static str {
        "grep"
    }

    fn description(&self) -> String {
        "搜索文件内容（正则匹配）。参数：pattern（正则）、path（搜索目录或文件，默认当前目录）、include（glob 过滤，可选）。".into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string"},
                "path": {"type": "string"},
                "include": {"type": "string"},
            },
            "required": ["pattern"],
        })
    }

    fn execute(&self, args: &Value, signal: &AtomicBool) -> Result<String, ToolError> {
        let pattern = arg_str(args, "pattern")?;
        let path = args.get("path").and_then(Value::as_str).unwrap_or(".");
        let include = args.get("include").and_then(Value::as_str);

        let mut cmd = Command::new("rg");
        cmd.arg("--json")
            .arg("--line-number")
            .arg("--no-heading")
            .arg("--max-count")
            .arg(MAX_MATCHES.to_string());

        if let Some(inc) = include {
            cmd.arg("--glob").arg(inc);
        }

        cmd.arg("--").arg(pattern).arg(path);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| ToolError::Message(format!("failed to spawn rg: {e}")))?;

        let output = child.stdout.take().unwrap();
        let deadline = Instant::now() + RG_TIMEOUT;

        // Read stdout in a separate thread to avoid deadlock
        let reader = std::thread::spawn(move || {
            use std::io::Read;
            let mut buf = Vec::new();
            let mut reader = std::io::BufReader::new(output);
            reader.read_to_end(&mut buf).unwrap_or(0);
            buf
        });

        // Poll for timeout / abort
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

        // Parse NDJSON: only "match" records
        let mut results = Vec::new();
        for line in stdout.lines() {
            if line.is_empty() {
                continue;
            }
            if let Ok(record) = serde_json::from_str::<Value>(line) {
                if record.get("type").and_then(Value::as_str) != Some("match") {
                    continue;
                }
                let data = match record.get("data") {
                    Some(d) => d,
                    None => continue,
                };
                let path_text = data
                    .get("path")
                    .and_then(|p| p.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or("?");
                let line_num = data.get("line_number").and_then(Value::as_u64).unwrap_or(0);
                let line_text = data
                    .get("lines")
                    .and_then(|l| l.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or("(non-utf8)")
                    .trim_end_matches('\n');
                results.push(format!("{path_text}:{line_num}: {line_text}"));
                if results.len() >= MAX_MATCHES {
                    break;
                }
            }
        }

        if results.is_empty() {
            return Ok("no matches found".into());
        }

        let header = format!("{} matches:\n", results.len());
        let body = results.join("\n");
        Ok(truncate_output(&format!("{header}{body}"), 0)?)
    }
}
