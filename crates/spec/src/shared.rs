//! Shared helpers for the gate validators.
//!
//! Centralizes the primitives every validator needs: a GitHub API client that
//! tolerates flaky networks, a Finding contract flowing through rule checks,
//! and a YAML loader.

use std::collections::BTreeMap;
use std::fs;
use std::process::Command;
use std::thread;
use std::time::Duration;

use serde_json::Value as JsonValue;
use serde_yaml::Value as YamlValue;

/// All short-lived network failures seen in this repo's CI history.
/// Non-matching errors (4xx, permission denied, malformed request) propagate
/// immediately — retrying them would just burn the budget.
pub const TRANSIENT_PATTERNS: &[&str] = &[
    "EOF",
    "unexpected EOF",
    "connection reset",
    "Connection reset",
    "Connection closed",
    "connection refused",
    "broken pipe",
    "TLS handshake timeout",
    "dial tcp",
    "i/o timeout",
    "net/http: timeout",
    "transport is closing",
    "500 Internal Server Error",
    "502 Bad Gateway",
    "503 Service Unavailable",
    "504 Gateway Timeout",
];

pub const MAX_RETRIES: u32 = 8;
pub const INITIAL_BACKOFF_SECONDS: u64 = 3;

/// Ordered so any `Fail` dominates `Warn`, which dominates `Info`.
/// IntEnum semantics: `Fail=10`, `Warn=20`, `Info=30`; lower sorts first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum Severity {
    Fail = 10,
    Warn = 20,
    Info = 30,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Fail => "FAIL",
            Severity::Warn => "WARN",
            Severity::Info => "INFO",
        }
    }
}

/// A single rule result.
/// `rule_id` is the stable identifier surfaced in CLI output (e.g. "P-30").
/// `line_hint` is optional because most checks operate on whole documents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub rule_id: String,
    pub severity: Severity,
    pub msg: String,
    pub line_hint: Option<u32>,
}

impl Finding {
    /// `{rule_id:<6} {SEVERITY}[\t{msg}]` with an optional ` L{line}` suffix
    /// on the prefix — matches the Python `Finding.format()` exactly.
    pub fn format(&self) -> String {
        let mut prefix = format!("{:<6} {}", self.rule_id, self.severity.as_str());
        if let Some(line) = self.line_hint {
            prefix.push_str(&format!(" L{}", line));
        }
        format!("{}\t{}", prefix, self.msg)
    }

    pub fn new(rule_id: &str, severity: Severity, msg: &str) -> Self {
        Self {
            rule_id: rule_id.to_string(),
            severity,
            msg: msg.to_string(),
            line_hint: None,
        }
    }

    pub fn with_line(mut self, line: u32) -> Self {
        self.line_hint = Some(line);
        self
    }
}

/// Return a "has failure" bool: true if any `Fail`.
/// The Python `aggregate_result` returns an exit code (1/0); callers needing
/// an exit code can use `aggregate_result(findings) as i32` via exit_code().
pub fn aggregate_result(findings: &[Finding]) -> bool {
    findings.iter().any(|f| f.severity == Severity::Fail)
}

/// Process exit code: 1 if any `Fail`, else 0 — the direct port of
/// `aggregate_result`.
pub fn exit_code(findings: &[Finding]) -> i32 {
    if aggregate_result(findings) { 1 } else { 0 }
}

/// Apply user-provided severity overrides to a list of findings.
/// Loads overrides from `.githooks/spec/severity_overrides.yaml` in the repo root.
pub fn apply_global_overrides(findings: &mut [Finding]) {
    let overrides = match load_severity_overrides() {
        Some(cfg) => cfg,
        None => return,
    };

    let map = match overrides
        .get("severity_overrides")
        .and_then(|c| c.as_mapping())
    {
        Some(m) => m,
        None => return,
    };
    for finding in findings.iter_mut() {
        if let Some(sev_val) = map.get(&YamlValue::String(finding.rule_id.to_string())) {
            if let Some(sev_str) = sev_val.as_str() {
                let new_sev = match sev_str.to_uppercase().as_str() {
                    "FAIL" => Severity::Fail,
                    "WARN" => Severity::Warn,
                    "INFO" => Severity::Info,
                    _ => continue,
                };
                if finding.severity != Severity::Info {
                    finding.severity = new_sev;
                }
            }
        }
    }
}
pub fn apply_severity_overrides(findings: &mut [Finding], cfg: Option<&YamlValue>) {
    let map = match cfg
        .and_then(|c| c.get("severity_overrides"))
        .and_then(|c| c.as_mapping())
    {
        Some(m) => m,
        None => return,
    };
    for finding in findings.iter_mut() {
        if let Some(sev_val) = map.get(&YamlValue::String(finding.rule_id.to_string())) {
            if let Some(sev_str) = sev_val.as_str() {
                let new_sev = match sev_str.to_uppercase().as_str() {
                    "FAIL" => Severity::Fail,
                    "WARN" => Severity::Warn,
                    "INFO" => Severity::Info,
                    _ => continue,
                };
                if finding.severity != Severity::Info {
                    finding.severity = new_sev;
                }
            }
        }
    }
}

/// Load severity_overrides from `.githooks/spec/severity_overrides.yaml`.
fn load_severity_overrides() -> Option<YamlValue> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let candidate = dir.join(".githooks");
        if candidate.is_dir() {
            let path = candidate.join("spec/severity_overrides.yaml");
            if let Ok(text) = fs::read_to_string(&path) {
                if let Ok(cfg) = serde_yaml::from_str(&text) {
                    return Some(cfg);
                }
            }
            return None;
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Print FAIL/WARN findings; suppress INFO unless no issues found.
/// All output goes to **stderr** — matches the Python which writes every line
/// to `sys.stderr`.
pub fn print_findings(findings: &[Finding]) {
    use std::io::Write;
    let stderr = std::io::stderr();
    let mut out = stderr.lock();

    let actionable: Vec<&Finding> = findings
        .iter()
        .filter(|f| f.severity <= Severity::Warn)
        .collect();
    if !actionable.is_empty() {
        let mut sorted = actionable.clone();
        sorted.sort_by(|a, b| {
            (a.severity, a.rule_id.as_str(), a.line_hint.unwrap_or(0)).cmp(&(
                b.severity,
                b.rule_id.as_str(),
                b.line_hint.unwrap_or(0),
            ))
        });
        for finding in &sorted {
            let _ = writeln!(out, "{}", finding.format());
        }
        let passed = findings
            .iter()
            .filter(|f| f.severity == Severity::Info)
            .count();
        if passed > 0 {
            let _ = writeln!(out, "({} checks passed)", passed);
        }
    } else {
        let _ = writeln!(out, "({} checks passed)", findings.len());
    }
    let _ = writeln!(
        out,
        "RESULT: {}",
        if aggregate_result(findings) {
            "FAIL"
        } else {
            "ALL PASS"
        }
    );
}

// ---------------------------------------------------------------------------
// Transient error detection
// ---------------------------------------------------------------------------

/// True if `message` contains any known transient-error substring.
pub fn is_transient(message: &str) -> bool {
    TRANSIENT_PATTERNS.iter().any(|pat| message.contains(pat))
}

// ---------------------------------------------------------------------------
// GitHub API client
// ---------------------------------------------------------------------------

/// Run `gh api <args...>`, returning (exit_code, combined_stripped_output).
/// All output is captured; combined stdout+stderr is trimmed.
fn run_gh(args: &[&str]) -> (i32, String) {
    let mut cmd = Command::new("gh");
    cmd.arg("api").args(args);
    let output = match cmd.output() {
        Ok(o) => o,
        Err(e) => return (127, e.to_string()),
    };
    let mut combined = String::new();
    combined.push_str(&String::from_utf8_lossy(&output.stdout));
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    (
        output.status.code().unwrap_or(127),
        combined.trim().to_string(),
    )
}

/// Run `gh api` with up to `MAX_RETRIES` attempts; backoff = `3s * attempt`.
/// Non-transient errors raise immediately; transient ones retry.
fn run_gh_with_retry(args: &[&str]) -> Result<String, String> {
    let mut last_msg = String::new();
    for attempt in 1..=MAX_RETRIES {
        let (rc, out) = run_gh(args);
        if rc == 0 {
            return Ok(out);
        }
        last_msg = out.clone();
        if !is_transient(&out) {
            return Err(format!("gh api hard failure ({}): {}", rc, out));
        }
        // Backoff: 3s * attempt.
        thread::sleep(Duration::from_secs(
            INITIAL_BACKOFF_SECONDS * attempt as u64,
        ));
    }
    Err(format!(
        "gh api exhausted {} retries: {}",
        MAX_RETRIES, last_msg
    ))
}

/// GET a GitHub REST endpoint, returning parsed JSON (or `Null` on empty).
/// `params` become `-F key=value` flags.
pub fn gh_api(path: &str, params: Option<&BTreeMap<&str, &str>>) -> Result<JsonValue, String> {
    let mut args: Vec<String> = vec![path.to_string()];
    if let Some(p) = params {
        for (k, v) in p {
            args.push("-F".to_string());
            args.push(format!("{}={}", k, v));
        }
    }
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let raw = run_gh_with_retry(&arg_refs)?;
    if raw.is_empty() {
        return Ok(JsonValue::Null);
    }
    serde_json::from_str(&raw).map_err(|e| format!("json decode: {}", e))
}

/// Run a GraphQL query via `gh api graphql`, returning the `data` payload.
/// When `variables` is given it is serialized to JSON and passed as
/// `-f variables=<json>` — the canonical way `gh api graphql` accepts them.
pub fn gh_api_graphql(query: &str, variables: Option<&JsonValue>) -> Result<JsonValue, String> {
    let field = format!("query={}", query);
    let mut args: Vec<String> = vec!["graphql".to_string(), "-f".to_string(), field];
    if let Some(vars) = variables {
        let json =
            serde_json::to_string(vars).map_err(|e| format!("json encode variables: {}", e))?;
        args.push("-f".to_string());
        args.push(format!("variables={}", json));
    }
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let raw = run_gh_with_retry(&arg_refs)?;
    if raw.is_empty() {
        return Ok(JsonValue::Null);
    }
    let parsed: JsonValue =
        serde_json::from_str(&raw).map_err(|e| format!("json decode: {}", e))?;
    if let Some(errors) = parsed.get("errors") {
        return Err(format!("GraphQL errors: {}", errors));
    }
    Ok(parsed.get("data").cloned().unwrap_or(JsonValue::Null))
}

/// Iterate a list endpoint by explicit cursor pagination.
/// Uses `page=N&per_page=N`; stops when a page returns fewer than `page_size`.
pub fn gh_api_paginate(path: &str, page_size: u32) -> Result<Vec<JsonValue>, String> {
    let mut results = Vec::new();
    let mut page = 1u32;
    loop {
        let sep = if path.contains('?') { '&' } else { '?' };
        let paged = format!("{}{}page={}&per_page={}", path, sep, page, page_size);
        let raw = run_gh_with_retry(&[&paged])?;
        if raw.is_empty() {
            break;
        }
        let items: Vec<JsonValue> =
            serde_json::from_str(&raw).map_err(|e| format!("json decode: {}", e))?;
        if items.is_empty() {
            break;
        }
        let len = items.len() as u32;
        results.extend(items);
        if len < page_size {
            break;
        }
        page += 1;
    }
    Ok(results)
}

// ---------------------------------------------------------------------------
// YAML loader
// ---------------------------------------------------------------------------

/// Load a YAML file as a `serde_yaml::Value`; empty/whitespace file → `Null`
/// (YAML null; callers treat `Null` as an empty map, mirroring Python's
/// `data or {}`). Returns an error if parsing fails or the file is missing.
pub fn load_yaml(path: &str) -> Result<YamlValue, serde_yaml::Error> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            return Err(<serde_yaml::Error as serde::de::Error>::custom(
                e.to_string(),
            ));
        }
    };
    if text.trim().is_empty() {
        return Ok(YamlValue::Null);
    }
    serde_yaml::from_str(&text)
}

// ---------------------------------------------------------------------------
// External command runner
// ---------------------------------------------------------------------------

/// Run an external command, returning (exit_code, combined_stripped_output).
pub fn run_external(cmd: &[&str], cwd: Option<&str>) -> Result<(i32, String), String> {
    if cmd.is_empty() {
        return Err("empty command".to_string());
    }
    let mut builder = Command::new(cmd[0]);
    builder.args(&cmd[1..]);
    if let Some(dir) = cwd {
        builder.current_dir(dir);
    }
    let output = builder
        .output()
        .map_err(|e| format!("spawn {:?}: {}", cmd[0], e))?;
    let mut combined = String::new();
    combined.push_str(&String::from_utf8_lossy(&output.stdout));
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok((
        output.status.code().unwrap_or(127),
        combined.trim().to_string(),
    ))
}

/// Truncate a string to at most `max` bytes without panicking on a multi-byte
/// UTF-8 boundary (CJK chars are multi-byte). Returns `s` unchanged if short.
pub fn truncate_utf8(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn finding_format_no_line() {
        let f = Finding::new("P-30", Severity::Warn, "missing section");
        assert_eq!(f.format(), "P-30   WARN\tmissing section");
    }

    #[test]
    fn finding_format_with_line() {
        let f = Finding::new("I-22b", Severity::Info, "ok").with_line(42);
        assert_eq!(f.format(), "I-22b  INFO L42\tok");
    }

    #[test]
    fn finding_format_fail_short_id_padding() {
        // rule_id shorter than 6 — Python uses {self.rule_id:<6} so "I-1" pads to width 6.
        let f = Finding::new("I-1", Severity::Fail, "boom");
        assert_eq!(f.format(), "I-1    FAIL\tboom");
    }

    #[test]
    fn severity_ordering() {
        assert!(Severity::Fail < Severity::Warn);
        assert!(Severity::Warn < Severity::Info);
        assert!(Severity::Fail < Severity::Info);
    }

    #[test]
    fn severity_as_str() {
        assert_eq!(Severity::Fail.as_str(), "FAIL");
        assert_eq!(Severity::Warn.as_str(), "WARN");
        assert_eq!(Severity::Info.as_str(), "INFO");
    }

    #[test]
    fn aggregate_result_detects_fail() {
        let findings = vec![
            Finding::new("A", Severity::Info, ""),
            Finding::new("B", Severity::Warn, ""),
        ];
        assert!(!aggregate_result(&findings));
        assert_eq!(exit_code(&findings), 0);

        let with_fail = vec![
            Finding::new("A", Severity::Warn, ""),
            Finding::new("B", Severity::Fail, ""),
        ];
        assert!(aggregate_result(&with_fail));
        assert_eq!(exit_code(&with_fail), 1);
    }

    #[test]
    fn print_findings_capture_actionable_and_pass_count() {
        // We can't easily capture stderr; instead drive the sort logic and
        // the pass-count computation by exercising the helpers directly, and
        // assert print_findings panics-free by running it.
        let findings = vec![
            Finding::new("Z-1", Severity::Info, "ok"),
            Finding::new("A-1", Severity::Warn, "w"),
            Finding::new("B-2", Severity::Fail, "f"),
            Finding::new("A-2", Severity::Fail, "f2"),
        ];
        // Sorted: Fail(B-2), Fail(A-2) -> by (severity, rule_id, line_hint)
        let actionable: Vec<&Finding> = findings
            .iter()
            .filter(|f| f.severity <= Severity::Warn)
            .collect();
        let mut sorted = actionable.clone();
        sorted.sort_by(|a, b| {
            (a.severity, a.rule_id.as_str(), a.line_hint.unwrap_or(0)).cmp(&(
                b.severity,
                b.rule_id.as_str(),
                b.line_hint.unwrap_or(0),
            ))
        });
        // Fail (severity 10) sorts before Warn (severity 20); both Fails sort by rule_id.
        assert_eq!(sorted[0].rule_id, "A-2");
        assert_eq!(sorted[0].severity, Severity::Fail);
        assert_eq!(sorted[1].rule_id, "B-2");
        assert_eq!(sorted[1].severity, Severity::Fail);
        assert_eq!(sorted[2].rule_id, "A-1");
        assert_eq!(sorted[2].severity, Severity::Warn);
        // Pass count = INFO findings = 1.
        assert_eq!(
            findings
                .iter()
                .filter(|f| f.severity == Severity::Info)
                .count(),
            1
        );
        // Should not panic:
        print_findings(&findings);
    }

    #[test]
    fn print_findings_only_info_shows_total() {
        let findings = vec![
            Finding::new("I-1", Severity::Info, "ok"),
            Finding::new("I-2", Severity::Info, "ok"),
        ];
        print_findings(&findings);
        // Smoke: not asserting stderr, just no panic.
    }

    #[test]
    fn load_yaml_parses_map() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spec.yaml");
        let mut fh = fs::File::create(&path).unwrap();
        writeln!(fh, "required_headings:").unwrap();
        writeln!(fh, "  - \"Goal\"").unwrap();
        writeln!(fh, "  - \"Done when\"").unwrap();
        drop(fh);
        let v = load_yaml(path.to_str().unwrap()).unwrap();
        let headings = v
            .get(serde_yaml::Value::String("required_headings".into()))
            .expect("required_headings present");
        let seq = headings.as_sequence().expect("sequence");
        assert_eq!(seq.len(), 2);
        assert_eq!(seq[0].as_str().unwrap(), "Goal");
    }

    #[test]
    fn load_yaml_empty_file_is_null() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.yaml");
        fs::write(&path, "").unwrap();
        let v = load_yaml(path.to_str().unwrap()).unwrap();
        assert!(v.is_null());
    }

    #[test]
    fn is_transient_detection() {
        assert!(is_transient("write tcp: connection reset by peer"));
        assert!(is_transient("HTTP 503 Service Unavailable"));
        assert!(is_transient("unexpected EOF"));
        assert!(is_transient("dial tcp 1.2.3.4: i/o timeout"));
        assert!(!is_transient("404 Not Found"));
        assert!(!is_transient("permission denied (publickey)"));
        assert!(!is_transient("401 Bad credentials"));
    }

    #[test]
    fn run_external_returns_code_and_output() {
        let (code, out) = run_external(&["true"], None).unwrap();
        assert_eq!(code, 0);
        assert!(out.is_empty() || out.trim().is_empty());
        let (code2, out2) = run_external(&["sh", "-c", "echo hi; echo err 1>&2"], None).unwrap();
        assert_eq!(code2, 0);
        // Combined stdout+stderr, trimmed.
        assert!(out2.contains("hi"));
        assert!(out2.contains("err"));
        let (code3, _out3) = run_external(&["sh", "-c", "exit 7"], None).unwrap();
        assert_eq!(code3, 7);
    }

    #[test]
    fn run_external_missing_binary_is_err() {
        let res = run_external(&["this-binary-does-not-exist-xyz123"], None);
        assert!(res.is_err());
    }

    #[test]
    fn truncate_utf8_ascii_short() {
        assert_eq!(truncate_utf8("hello", 10), "hello");
        assert_eq!(truncate_utf8("", 5), "");
    }

    #[test]
    fn truncate_utf8_ascii_cut() {
        assert_eq!(truncate_utf8("hello world", 5), "hello");
    }

    #[test]
    fn truncate_utf8_cjk_boundary_safe() {
        // "中文abc" = 2×3 + 3 = 9 bytes; cut at 6 lands exactly at char end.
        assert_eq!(truncate_utf8("中文abc", 6), "中文");
        // cut at 5 lands mid-CJK (second char) → truncated to first char.
        assert_eq!(truncate_utf8("中文abc", 5), "中");
        // max==0 on non-empty input → empty string, no panic.
        assert_eq!(truncate_utf8("中文", 0), "");
        // Boundary exactly at char end keeps the CJK char.
        assert_eq!(truncate_utf8("中文abc", 9), "中文abc");
    }

    #[test]
    fn truncate_utf8_mid_3byte_char() {
        // "严" is U+4E25, 3 bytes (e4 b8 a5). max=1 lands mid-byte → "".
        assert_eq!(truncate_utf8("严", 1), "");
        assert_eq!(truncate_utf8("严", 3), "严");
    }
}
