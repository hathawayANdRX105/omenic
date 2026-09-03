//! Project-level LLM checklist dispatcher: CK-01.
//!
//! Each `checklist_*.yaml` under `.githooks/spec/` declares one project
//! check. Gate pipes the relevant git diff (or each changed file's
//! contents) to a user-defined harness (any executable) and parses the
//! stdout JSON array back into `Finding`s. Severity is max(yaml-level,
//! harness-reported) so a FAIL from the harness always blocks.
//!
//! See `.githooks/spec/CHECKLIST_SPEC.md` for the full yaml schema and
//! harness protocol.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde::Deserialize;

use crate::shared::{Finding, Severity, load_yaml};
use crate::tools::git;

/// Which gate entrypoint invoked us; controls the diff scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookScope {
    PreCommit,
    PrePush,
    Merge,
}

impl HookScope {
    fn as_str(self) -> &'static str {
        match self {
            HookScope::PreCommit => "pre-commit",
            HookScope::PrePush => "pre-push",
            HookScope::Merge => "merge",
        }
    }

    fn matches_yaml(self, yaml_hook: &str) -> bool {
        yaml_hook == self.as_str()
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawSpec {
    enabled: Option<bool>,
    hooks: Vec<String>,
    #[serde(default)]
    r#match: MatchSpec,
    mode: Option<String>,
    harness: HarnessSpec,
    timeout: Option<u64>,
    optional: Option<bool>,
    fail_severity: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct MatchSpec {
    paths_include: Vec<String>,
    paths_exclude: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct HarnessSpec {
    command: String,
    args: Vec<String>,
}

#[derive(Debug)]
struct ChecklistSpec {
    name: String,
    enabled: bool,
    hooks: Vec<String>,
    include: Vec<String>,
    exclude: Vec<String>,
    mode: Mode,
    command: String,
    args: Vec<String>,
    timeout_secs: u64,
    optional: bool,
    base_severity: Severity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Diff,
    File,
    /// Static check: harness receives empty stdin and runs whatever
    /// grep/find/ripgrep/etc. it wants. Findings carry their own
    /// path/line via the JSON output.
    Grep,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum HarnessFinding {
    Single(FindingJson),
    Many(Vec<FindingJson>),
}

#[derive(Debug, Deserialize)]
struct FindingJson {
    id: String,
    severity: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    line: Option<u32>,
    message: String,
}

// ---------------------------------------------------------------------------
// Spec loading
// ---------------------------------------------------------------------------

fn parse_severity(s: &str) -> Option<Severity> {
    match s.to_ascii_uppercase().as_str() {
        "FAIL" => Some(Severity::Fail),
        "WARN" => Some(Severity::Warn),
        "INFO" => Some(Severity::Info),
        _ => None,
    }
}

fn load_spec(path: &std::path::Path) -> Option<ChecklistSpec> {
    let v = match load_yaml(path.to_str()?) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("checklist: skip (yaml parse fail {}): {e}", path.display());
            return None;
        }
    };
    let raw: RawSpec = match serde_yaml::from_value(v.clone()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("checklist: deserialize fail {}: {e}", path.display());
            return None;
        }
    };
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .strip_prefix("checklist_")
        .unwrap_or("unknown")
        .to_string();
    let enabled = raw.enabled.unwrap_or(true);
    let mode = match raw.mode.as_deref().unwrap_or("diff") {
        "file" => Mode::File,
        "grep" => Mode::Grep,
        _ => Mode::Diff,
    };
    let base_severity = raw
        .fail_severity
        .as_deref()
        .and_then(parse_severity)
        .unwrap_or(Severity::Warn);
    Some(ChecklistSpec {
        name,
        enabled,
        hooks: if raw.hooks.is_empty() {
            vec!["pre-commit".into(), "pre-push".into(), "merge".into()]
        } else {
            raw.hooks
        },
        include: raw.r#match.paths_include,
        exclude: raw.r#match.paths_exclude,
        mode,
        command: raw.harness.command,
        args: raw.harness.args,
        timeout_secs: raw.timeout.unwrap_or(60),
        optional: raw.optional.unwrap_or(true),
        base_severity,
    })
}

fn find_specs(spec_dir: &std::path::Path) -> Vec<(PathBuf, ChecklistSpec)> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(spec_dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("yaml") {
            continue;
        }
        if path
            .file_name()
            .and_then(|s| s.to_str())
            .is_none_or(|n| !n.starts_with("checklist_"))
        {
            continue;
        }
        if let Some(spec) = load_spec(&path) {
            out.push((path, spec));
        }
    }
    // Stable order: file name.
    out.sort_by(|a, b| a.0.file_name().cmp(&b.0.file_name()));
    out
}

// ---------------------------------------------------------------------------
// Diff / file plumbing
// ---------------------------------------------------------------------------

fn diff_args(scope: HookScope) -> Vec<&'static str> {
    match scope {
        HookScope::PreCommit => vec!["diff", "--cached", "--unified=3", "--no-color"],
        HookScope::PrePush => vec!["diff", "HEAD", "--unified=3", "--no-color"],
        // origin/main..HEAD — assume main as the merge base ref; user can rename
        // via upstream remote if needed. Future: read from config.
        HookScope::Merge => vec!["diff", "origin/main...HEAD", "--unified=3", "--no-color"],
    }
}

fn capture_diff(scope: HookScope) -> Option<String> {
    let out = Command::new("git").args(diff_args(scope)).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

fn changed_files(scope: HookScope) -> Vec<String> {
    let args: Vec<&str> = match scope {
        HookScope::PreCommit => vec!["diff", "--cached", "--name-only", "--no-color"],
        HookScope::PrePush => vec!["diff", "HEAD", "--name-only", "--no-color"],
        HookScope::Merge => vec!["diff", "origin/main...HEAD", "--name-only", "--no-color"],
    };
    let Ok(out) = Command::new("git").args(args).output() else {
        return vec![];
    };
    if !out.status.success() {
        return vec![];
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn matches_include(rel: &str, include: &str) -> bool {
    // Mirrors the heuristic used by `code.rs::matches_include`; the harness
    // will see the full file content, so a loose match is fine here.
    let pat = include.strip_prefix("**/").unwrap_or(include);
    if let Some(ext) = pat.strip_prefix("*.") {
        return rel.ends_with(&format!(".{ext}"));
    }
    if let Some(suffix) = pat.strip_prefix('*') {
        return rel.ends_with(suffix);
    }
    rel == pat || rel.contains(&format!("/{pat}"))
}

fn file_matches(spec: &ChecklistSpec, rel: &str) -> bool {
    if !spec.exclude.iter().all(|x| !rel.contains(x)) {
        return false;
    }
    if spec.include.is_empty() {
        return true;
    }
    spec.include.iter().any(|p| matches_include(rel, p))
}

fn has_match(spec: &ChecklistSpec, scope: HookScope) -> bool {
    let files = changed_files(scope);
    files.iter().any(|f| file_matches(spec, f))
}

// ---------------------------------------------------------------------------
// Harness invocation
// ---------------------------------------------------------------------------

fn run_harness(spec: &ChecklistSpec, stdin_payload: &[u8]) -> (i32, String) {
    let mut cmd = Command::new(&spec.command);
    cmd.args(&spec.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return (127, String::new()),
    };
    if let Some(mut sin) = child.stdin.take() {
        let _ = sin.write_all(stdin_payload);
    }
    match child.wait_with_output() {
        Ok(out) => {
            let code = out.status.code().unwrap_or(-1);
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            (code, combined)
        }
        Err(_) => (1, String::new()),
    }
}

fn merge_severity(base: Severity, reported: Severity) -> Severity {
    // Severity order: Fail < Warn < Info. Max-wins means smaller ordinal.
    std::cmp::min(base, reported)
}

fn findings_from_stdout(spec: &ChecklistSpec, stdout: &str) -> Vec<Finding> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() || trimmed == "[]" {
        return vec![];
    }
    // Try as a bare array first; if that fails, try a single object.
    let parsed: Result<HarnessFinding, _> = serde_json::from_str(trimmed);
    let items: Vec<FindingJson> = match parsed {
        Ok(HarnessFinding::Many(v)) => v,
        Ok(HarnessFinding::Single(s)) => vec![s],
        Err(_) => {
            // Maybe harness wrapped output — try the last JSON array on a line.
            if let Some(start) = trimmed.rfind('[') {
                if let Some(end) = trimmed.rfind(']') {
                    if end > start {
                        if let Ok(HarnessFinding::Many(v)) =
                            serde_json::from_str(&trimmed[start..=end])
                        {
                            return convert(spec, v);
                        }
                    }
                }
            }
            eprintln!(
                "checklist.{}: harness stdout not valid JSON; first 80 chars: {}",
                spec.name,
                truncate(trimmed, 80)
            );
            return vec![Finding::new(
                &format!("checklist.{}.CK-01", spec.name),
                Severity::Warn,
                "harness output not valid JSON; check skipped",
            )];
        }
    };
    convert(spec, items)
}

fn convert(spec: &ChecklistSpec, items: Vec<FindingJson>) -> Vec<Finding> {
    items
        .into_iter()
        .filter_map(|raw| {
            let reported = parse_severity(&raw.severity)?;
            let severity = merge_severity(spec.base_severity, reported);
            let id = format!("checklist.{}.{}", spec.name, raw.id);
            let path_prefix = raw
                .path
                .as_deref()
                .filter(|p| !p.is_empty())
                .map(|p| format!("{p}: "))
                .unwrap_or_default();
            let line_suffix = raw.line.map(|l| format!(" (L{l})")).unwrap_or_default();
            let msg = format!("{path_prefix}{}{line_suffix}", raw.message);
            let mut f = Finding::new(&id, severity, &msg);
            if let Some(l) = raw.line {
                f = f.with_line(l);
            }
            Some(f)
        })
        .collect()
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        // Find a char boundary at or before `max` to avoid mid-codepoint cuts.
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &s[..end])
    }
}

// ---------------------------------------------------------------------------
// Per-spec execution
// ---------------------------------------------------------------------------

fn run_one(spec: &ChecklistSpec, scope: HookScope) -> Vec<Finding> {
    if !spec.enabled {
        return vec![Finding::new(
            &format!("checklist.{}", spec.name),
            Severity::Info,
            "disabled in config",
        )];
    }
    if !spec.hooks.iter().any(|h| scope.matches_yaml(h)) {
        return vec![]; // not in this hook's scope — silent skip
    }
    if spec.mode != Mode::Grep && !has_match(spec, scope) {
        return vec![Finding::new(
            &format!("checklist.{}", spec.name),
            Severity::Info,
            "no matching files in diff",
        )];
    }

    let stdin_payload: Vec<u8> = match spec.mode {
        Mode::Diff => capture_diff(scope).unwrap_or_default().into_bytes(),
        Mode::File => {
            // Concatenate all matching changed files; harness gets a clear
            // separator so it can attribute findings back to a file.
            let root = git::git_root().unwrap_or_else(|| PathBuf::from("."));
            let mut buf = String::new();
            for rel in changed_files(scope) {
                if !file_matches(spec, &rel) {
                    continue;
                }
                let path = root.join(&rel);
                if let Ok(content) = std::fs::read_to_string(&path) {
                    buf.push_str(&format!("\n===== FILE: {rel} =====\n"));
                    buf.push_str(&content);
                }
            }
            buf.into_bytes()
        }
        Mode::Grep => Vec::new(), // harness runs static checks itself
    };

    if stdin_payload.is_empty() && spec.mode != Mode::Grep {
        return vec![Finding::new(
            &format!("checklist.{}", spec.name),
            Severity::Info,
            "empty diff; nothing to check",
        )];
    }

    let (rc, output) = run_harness(spec, &stdin_payload);
    if rc == 127 || output.to_lowercase().contains("no such file or directory") {
        let sev = if spec.optional {
            Severity::Warn
        } else {
            Severity::Fail
        };
        return vec![Finding::new(
            &format!("checklist.{}", spec.name),
            sev,
            &format!("harness not installed: {} (skipped)", spec.command),
        )];
    }
    if rc != 0 && rc != 2 {
        // Unexpected non-zero — surface as WARN with exit code.
        return vec![Finding::new(
            &format!("checklist.{}", spec.name),
            Severity::Warn,
            &format!("harness exited {rc}: {}", truncate(&output, 200)),
        )];
    }
    if rc == 2 {
        return vec![Finding::new(
            &format!("checklist.{}", spec.name),
            Severity::Warn,
            &format!("harness internal error: {}", truncate(&output, 200)),
        )];
    }

    findings_from_stdout(spec, &output)
}

/// Run all `checklist_*.yaml` matching the scope. Returns findings aggregated
/// across every spec; the caller (`pre_commit` / `pre_push` / `merge`)
/// appends them to the global finding list and prints via the shared
/// `print_findings`.
pub fn run_all(scope: HookScope) -> Vec<Finding> {
    let githooks = git::find_githooks_dir().unwrap_or_else(|| PathBuf::from(".githooks"));
    let spec_dir = githooks.join("spec");
    let specs = find_specs(&spec_dir);
    if specs.is_empty() {
        return vec![];
    }
    let mut findings = Vec::new();
    for (_path, spec) in &specs {
        eprintln!("--- checklist: {} ---", spec.name);
        findings.extend(run_one(spec, scope));
    }
    findings
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_severity_basic() {
        assert_eq!(parse_severity("FAIL"), Some(Severity::Fail));
        assert_eq!(parse_severity("warn"), Some(Severity::Warn));
        assert_eq!(parse_severity("INFO"), Some(Severity::Info));
        assert_eq!(parse_severity("nope"), None);
    }

    #[test]
    fn merge_severity_takes_worse() {
        // base WARN, reported FAIL → FAIL
        assert_eq!(
            merge_severity(Severity::Warn, Severity::Fail),
            Severity::Fail
        );
        // base FAIL, reported INFO → FAIL (cannot downgrade)
        assert_eq!(
            merge_severity(Severity::Fail, Severity::Info),
            Severity::Fail
        );
        // base INFO, reported WARN → WARN
        assert_eq!(
            merge_severity(Severity::Info, Severity::Warn),
            Severity::Warn
        );
    }

    #[test]
    fn convert_one_finding_keeps_fail_even_with_warn_base() {
        let spec = ChecklistSpec {
            name: "demo".into(),
            enabled: true,
            hooks: vec![],
            include: vec![],
            exclude: vec![],
            mode: Mode::Diff,
            command: "x".into(),
            args: vec![],
            timeout_secs: 60,
            optional: true,
            base_severity: Severity::Warn,
        };
        let items = vec![FindingJson {
            id: "X-01".into(),
            severity: "FAIL".into(),
            path: Some("src/foo.rs".into()),
            line: Some(42),
            message: "boom".into(),
        }];
        let out = convert(&spec, items);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].rule_id, "checklist.demo.X-01");
        assert_eq!(out[0].severity, Severity::Fail);
        assert_eq!(out[0].line_hint, Some(42));
        assert!(out[0].msg.contains("src/foo.rs:"));
        assert!(out[0].msg.contains("boom"));
    }

    #[test]
    fn convert_empty_path_no_prefix() {
        let spec = ChecklistSpec {
            name: "demo".into(),
            enabled: true,
            hooks: vec![],
            include: vec![],
            exclude: vec![],
            mode: Mode::Diff,
            command: "x".into(),
            args: vec![],
            timeout_secs: 60,
            optional: true,
            base_severity: Severity::Info,
        };
        let out = convert(
            &spec,
            vec![FindingJson {
                id: "G-01".into(),
                severity: "WARN".into(),
                path: None,
                line: None,
                message: "global".into(),
            }],
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].msg, "global");
    }

    #[test]
    fn findings_from_stdout_parses_array() {
        let spec = ChecklistSpec {
            name: "x".into(),
            enabled: true,
            hooks: vec![],
            include: vec![],
            exclude: vec![],
            mode: Mode::Diff,
            command: "x".into(),
            args: vec![],
            timeout_secs: 60,
            optional: true,
            base_severity: Severity::Info,
        };
        let out = findings_from_stdout(&spec, r#"[{"id":"A-1","severity":"WARN","message":"hi"}]"#);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].msg, "hi");
    }

    #[test]
    fn findings_from_stdout_empty_array_is_pass() {
        let spec = ChecklistSpec {
            name: "x".into(),
            enabled: true,
            hooks: vec![],
            include: vec![],
            exclude: vec![],
            mode: Mode::Diff,
            command: "x".into(),
            args: vec![],
            timeout_secs: 60,
            optional: true,
            base_severity: Severity::Info,
        };
        let out = findings_from_stdout(&spec, "[]");
        assert!(out.is_empty());
    }

    #[test]
    fn findings_from_stdout_garbage_is_warn_skip() {
        let spec = ChecklistSpec {
            name: "x".into(),
            enabled: true,
            hooks: vec![],
            include: vec![],
            exclude: vec![],
            mode: Mode::Diff,
            command: "x".into(),
            args: vec![],
            timeout_secs: 60,
            optional: true,
            base_severity: Severity::Info,
        };
        let out = findings_from_stdout(&spec, "not json {");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].severity, Severity::Warn);
        assert!(out[0].msg.contains("not valid JSON"));
    }

    #[test]
    fn file_matches_respects_exclude() {
        let spec = ChecklistSpec {
            name: "x".into(),
            enabled: true,
            hooks: vec![],
            include: vec!["**/*.rs".into()],
            exclude: vec!["target/".into(), ".wt/".into()],
            mode: Mode::Diff,
            command: "x".into(),
            args: vec![],
            timeout_secs: 60,
            optional: true,
            base_severity: Severity::Info,
        };
        assert!(file_matches(&spec, "crates/page/admin/src/network.rs"));
        assert!(!file_matches(&spec, "target/debug/foo.rs"));
        assert!(!file_matches(&spec, ".wt/admin/foo.rs"));
    }

    #[test]
    fn load_spec_parses_minimal_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("checklist_my_demo.yaml");
        std::fs::write(
            &path,
            r#"
enabled: true
hooks: [pre-push, merge]
mode: file
harness:
  command: "echo"
  args: ["hello"]
timeout: 30
optional: false
fail_severity: FAIL
match:
  paths_include: ["**/*.rs"]
  paths_exclude: ["target/"]
"#,
        )
        .unwrap();
        let spec = load_spec(&path).expect("load ok");
        assert_eq!(spec.name, "my_demo");
        assert!(spec.enabled);
        assert_eq!(spec.hooks, vec!["pre-push", "merge"]);
        assert_eq!(spec.mode, Mode::File);
        assert_eq!(spec.command, "echo");
        assert_eq!(spec.args, vec!["hello"]);
        assert_eq!(spec.timeout_secs, 30);
        assert!(!spec.optional);
        assert_eq!(spec.base_severity, Severity::Fail);
        assert_eq!(spec.include, vec!["**/*.rs".to_string()]);
        assert_eq!(spec.exclude, vec!["target/".to_string()]);
    }

    #[test]
    fn truncate_handles_mid_codepoint() {
        // CJK char at boundary
        assert!(!truncate("中文abc", 3).is_empty());
        // No panic on empty
        assert_eq!(truncate("", 5), "");
    }

    #[test]
    fn load_spec_parses_grep_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("checklist_static.yaml");
        std::fs::write(
            &path,
            r#"
enabled: true
mode: grep
harness:
  command: "sh"
  args: ["-c", "echo '[]'"]
optional: true
"#,
        )
        .unwrap();
        let spec = load_spec(&path).expect("load ok");
        assert_eq!(spec.mode, Mode::Grep);
    }

    #[test]
    fn load_spec_defaults_to_diff_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("checklist_default.yaml");
        std::fs::write(
            &path,
            r#"
enabled: true
harness:
  command: "true"
"#,
        )
        .unwrap();
        let spec = load_spec(&path).expect("load ok");
        assert_eq!(spec.mode, Mode::Diff);
    }

    #[test]
    fn grep_mode_does_not_short_circuit_on_empty_stdin() {
        // Mode::Grep always has empty stdin, but the harness runs
        // static checks itself. The dispatcher must NOT return
        // "empty diff; nothing to check" for grep mode.
        // We can't easily exercise run_one() here, but we can at
        // least verify the behavior at the spec level by reading
        // mode parsing for grep.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("checklist_grep_no_stdin.yaml");
        std::fs::write(
            &path,
            r#"
mode: grep
harness:
  command: "echo"
  args: ["[{\"id\":\"G-01\",\"severity\":\"WARN\",\"line\":1,\"message\":\"from grep\"}]"]
"#,
        )
        .unwrap();
        let spec = load_spec(&path).expect("load ok");
        assert_eq!(spec.mode, Mode::Grep);
        // The structural condition (don't skip on empty stdin
        // when mode == Grep) is exercised in run_one; the test
        // here just guards against regressing the Mode::Grep
        // recognition path.
    }
}
