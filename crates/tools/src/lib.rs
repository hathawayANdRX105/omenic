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
use serde_json::Value;

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

/// Allow or deny outcome for one tool invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
}

/// One policy rule: an exact tool name plus a literal substring of that
/// tool's subject (its command for `run_bash`, its path otherwise). Both
/// must match for the rule to apply.
#[derive(Debug, Clone)]
pub struct Rule {
    pub tool: String,
    /// Literal substring the subject must contain, tested with
    /// `str::contains`: no globbing, no path normalization, no shell
    /// parsing. Empty matches any subject.
    pub contains: String,
    pub decision: Decision,
    /// Human-readable justification, echoed in the denial message.
    pub reason: String,
}

impl Rule {
    pub fn deny(tool: &str, contains: &str, reason: &str) -> Self {
        Self {
            tool: tool.to_string(),
            contains: contains.to_string(),
            decision: Decision::Deny,
            reason: reason.to_string(),
        }
    }

    pub fn allow(tool: &str, contains: &str, reason: &str) -> Self {
        Self {
            tool: tool.to_string(),
            contains: contains.to_string(),
            decision: Decision::Allow,
            reason: reason.to_string(),
        }
    }
}

/// Synchronous allow/deny policy: ordered rules over a default decision.
/// First matching rule wins; unmatched invocations fall back to `default`.
///
/// Matching is literal substring containment on the subject, which is a
/// coarse filter and not a containment boundary: `contains: "src/"` also
/// matches `../src/x` and `/etc/src/passwd`, and no substring test survives
/// quoting, `$(…)`, `;` or `PATH` tricks in a bash command. Use it to catch
/// obvious mistakes; anything that must genuinely be confined needs an
/// out-of-band confirmation step as well.
///
// ponytail: substring matching is the entire engine. Security-grade
// confinement needs a real allowlist plus per-tool parsers — canonicalized
// path prefixes under a root for the file tools, argv-level command
// allowlisting for run_bash. Upgrade when this policy has to hold against an
// adversarial caller rather than an erring one.
#[derive(Debug, Clone)]
pub struct Policy {
    pub default: Decision,
    pub rules: Vec<Rule>,
}

impl Policy {
    pub fn new(default: Decision) -> Self {
        Self {
            default,
            rules: Vec::new(),
        }
    }

    /// Permit everything — the behavior `builtin_tools()` has always had.
    pub fn allow_all() -> Self {
        Self::new(Decision::Allow)
    }

    pub fn deny_all() -> Self {
        Self::new(Decision::Deny)
    }

    /// Append a rule; rules are evaluated in insertion order.
    pub fn rule(mut self, rule: Rule) -> Self {
        self.rules.push(rule);
        self
    }

    /// Judge `tool` acting on `subject`. Denials carry the tool name and the
    /// reason of the rule that rejected it. `subject` is compared by literal
    /// substring containment — see the caveat on [`Policy`].
    pub fn check(&self, tool: &str, subject: &str) -> Result<(), ToolError> {
        let (decision, reason) = match self
            .rules
            .iter()
            .find(|r| r.tool == tool && subject.contains(&r.contains))
        {
            Some(r) => (r.decision, r.reason.as_str()),
            None => (self.default, "no matching rule"),
        };
        match decision {
            Decision::Allow => Ok(()),
            Decision::Deny => Err(ToolError::Message(format!(
                "{tool} denied by permission policy: {reason}"
            ))),
        }
    }
}

/// Argument key holding the subject a rule matches against, plus the subject
/// to judge when that argument is absent or not a string.
///
/// Private, and closed over the built-in argument shapes on purpose: wrapping
/// a third-party tool in [`Guarded`] must not look guarded when its subject
/// lives under some other key. An unknown tool maps to no key, so it is
/// judged on an empty subject and a deny default rejects it.
fn subject_key(tool: &str) -> (&'static str, &'static str) {
    match tool {
        "run_bash" => ("command", ""),
        // grep and glob take an optional path; rg searches the cwd without it
        "grep" | "glob" => ("path", "."),
        "read_file" | "write_file" | "edit" | "delete_file" => ("path", ""),
        _ => ("", ""),
    }
}

/// Tool wrapper that consults a [`Policy`] before delegating to `execute`.
/// Transparent otherwise: name, description and schema are the inner tool's.
/// The check runs first, so a denial lands before any side effect.
///
/// Only meaningful for the built-in tools: the subject mapping is private and
/// keyed on built-in tool names, so an unknown inner tool is judged on an
/// empty subject and is rejected outright under a deny default.
pub struct Guarded<T> {
    inner: T,
    policy: Policy,
}

impl<T: Tool> Guarded<T> {
    pub fn new(inner: T, policy: Policy) -> Self {
        Self { inner, policy }
    }
}

impl<T: Tool> Tool for Guarded<T> {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn description(&self) -> String {
        self.inner.description()
    }

    fn parameters(&self) -> Value {
        self.inner.parameters()
    }

    fn execute(&self, args: &Value, signal: &AtomicBool) -> Result<String, ToolError> {
        let name = self.inner.name();
        let (key, absent) = subject_key(name);
        // Fail closed: a missing or non-string subject is still judged, using
        // the tool's default subject, so a deny default cannot be sidestepped
        // by omitting the argument.
        let subject = args.get(key).and_then(Value::as_str).unwrap_or(absent);
        self.policy.check(name, subject)?;
        self.inner.execute(args, signal)
    }
}

/// Register all built-in tools, unrestricted.
pub fn builtin_tools() -> Vec<Box<dyn Tool>> {
    builtin_tools_with_policy(Policy::allow_all())
}

/// Register all built-in tools, every one routed through `policy` so a
/// `deny_all` default applies uniformly. The read-only tools are wrapped too;
/// a policy that wants them permitted says so with an allow default or an
/// explicit allow rule.
pub fn builtin_tools_with_policy(policy: Policy) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(Guarded::new(read::ReadFile, policy.clone())),
        Box::new(Guarded::new(write::WriteFile, policy.clone())),
        Box::new(Guarded::new(edit::EditFile, policy.clone())),
        Box::new(Guarded::new(bash::RunBash, policy.clone())),
        Box::new(Guarded::new(grep::Grep, policy.clone())),
        Box::new(Guarded::new(glob::Glob, policy.clone())),
        Box::new(Guarded::new(delete::DeleteFile, policy)),
    ]
}
