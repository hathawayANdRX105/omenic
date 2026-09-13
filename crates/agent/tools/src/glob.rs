//! glob tool: list files matching a glob pattern using `rg --files`.
//!
//! Runs `rg --files --glob=<pattern> --sort=modified` as a subprocess,
//! returns matching paths. Respects gitignore by default.

use std::process::Command;

use serde_json::{Value, json};

use crate::{Tool, ToolError, arg_str, run_subprocess, truncate_output};

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

    fn execute(
        &self,
        args: &Value,
        signal: &std::sync::atomic::AtomicBool,
    ) -> Result<String, ToolError> {
        let pattern = arg_str(args, "pattern")?;
        let path = args.get("path").and_then(Value::as_str).unwrap_or(".");

        let mut cmd = Command::new("rg");
        cmd.arg("--files")
            .arg("--sort=modified")
            .arg(format!("--glob={pattern}"))
            .arg("--")
            .arg(path);

        let raw = run_subprocess(cmd, signal)?;
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
