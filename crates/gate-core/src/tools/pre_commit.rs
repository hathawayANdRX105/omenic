//! pre-commit hook — commit title validation (CM-01/02/03) + workspace + code checks.
//!
//! Port of `.githooks/hooks/pre-commit`. Runs the topics listed in
//! `dispatch.yaml` under `pre-commit` plus the commit-title checks that
//! the Python hook does unconditionally.
//!
//! CM-* checks are native Rust. Topic validators (workspace, code) are
//! delegated to the existing Python scripts under `.githooks/`.

use regex::Regex;
use std::sync::LazyLock;

use crate::shared::{Finding, Severity, exit_code, print_findings, run_external};
use crate::tools::git;

// ---------------------------------------------------------------------------
// Commit title validation (CM-01, CM-02, CM-03)
// ---------------------------------------------------------------------------

const CONV_COMMIT_TYPES: &[&str] = &[
    "feat", "fix", "docs", "style", "refactor", "perf", "test", "build", "ci", "chore", "revert",
];

static CONV_COMMIT_RE: LazyLock<Regex> = LazyLock::new(|| {
    let types = CONV_COMMIT_TYPES.join("|");
    Regex::new(&format!(r"^(?:{types})(?:\(.+\))?!?:\s+\S+")).unwrap()
});

static CJK_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\u{4e00}-\u{9fff}]").unwrap());

static TYPE_RE: LazyLock<Regex> = LazyLock::new(|| {
    let types = CONV_COMMIT_TYPES.join("|");
    Regex::new(&format!(r"^({types})")).unwrap()
});

fn check_commit_title(title: &str) -> Vec<Finding> {
    let mut findings = Vec::new();

    // Skip auto-generated messages
    if title.is_empty()
        || title.starts_with("Merge ")
        || title.starts_with("Revert ")
        || title.starts_with("fixup!")
        || title.starts_with("squash!")
    {
        return findings;
    }

    if !CONV_COMMIT_RE.is_match(title) {
        findings.push(Finding::new(
            "CM-01",
            Severity::Fail,
            &format!("commit title not conventional commit format: '{}'", title),
        ));
    } else if CJK_RE.is_match(title) {
        findings.push(Finding::new(
            "CM-02",
            Severity::Fail,
            &format!("commit title contains CJK (should be English): '{}'", title),
        ));
    }

    findings
}

fn check_commit_pr_consistency() -> Vec<Finding> {
    let repo = match git::derive_repo() {
        Some(r) => r,
        None => return vec![],
    };
    let branch = match git::current_branch() {
        Some(b) if b != "main" && b != "master" => b,
        _ => return vec![],
    };

    let pr_num = match git::find_pr_for_branch(&repo, &branch) {
        Some(n) => n,
        None => return vec![],
    };

    let path = format!("repos/{}/pulls/{}", repo, pr_num);
    let pr_json = match crate::shared::gh_api(&path, None) {
        Ok(j) => j,
        Err(_) => return vec![],
    };
    let pr_title = pr_json.get("title").and_then(|t| t.as_str()).unwrap_or("");
    let pr_type = match TYPE_RE
        .captures(pr_title)
        .map(|c| c.get(1).unwrap().as_str().to_string())
    {
        Some(t) => t,
        None => return vec![],
    };

    let commit_title = match git::read_commit_editmsg() {
        Some(t) => t.lines().next().unwrap_or("").trim().to_string(),
        None => return vec![],
    };
    let commit_type = TYPE_RE
        .captures(&commit_title)
        .map(|c| c.get(1).unwrap().as_str().to_string());
    if let Some(ct) = commit_type {
        if ct != pr_type {
            return vec![Finding::new(
                "CM-03",
                Severity::Fail,
                &format!(
                    "commit type '{}' != PR #{} type '{}'\n  commit: {}\n  PR: {}",
                    ct, pr_num, pr_type, commit_title, pr_title
                ),
            )];
        }
    }

    vec![]
}

// ---------------------------------------------------------------------------
// Python topic delegation
// ---------------------------------------------------------------------------

/// Run a Python script under `.githooks/`, parse its stdout for FAIL/RESULT lines,
/// and convert to Findings.
pub fn run_python_topic(script_rel: &str) -> Vec<Finding> {
    let githooks = match git::find_githooks_dir() {
        Some(d) => d,
        None => return vec![Finding::new("topic", Severity::Warn, ".githooks not found")],
    };
    let script = githooks.join(script_rel);
    let cwd = githooks.parent().map(|p| p.to_string_lossy().to_string());

    match run_external(&["python3", script.to_str().unwrap_or("")], cwd.as_deref()) {
        Ok((rc, output)) => parse_topic_output(&output, rc),
        Err(e) => vec![Finding::new(
            "topic",
            Severity::Warn,
            &format!("{}: {}", script_rel, e),
        )],
    }
}

/// Parse Python script output into Findings: FAIL lines become Fail findings,
/// RESULT: FAIL becomes a summary, RESULT: ALL PASS or 0 exit → INFO.
fn parse_topic_output(output: &str, rc: i32) -> Vec<Finding> {
    let mut findings = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        // Lines like "CM-01  FAIL\tmessage"
        if let Some(finding) = parse_finding_line(trimmed) {
            findings.push(finding);
        }
    }
    if findings.is_empty() && rc == 0 {
        findings.push(Finding::new("topic", Severity::Info, "passed"));
    } else if findings.is_empty() && rc != 0 {
        findings.push(Finding::new(
            "topic",
            Severity::Warn,
            &format!("exit {} with no parseable findings", rc),
        ));
    }
    findings
}

/// Parse a single output line into a Finding if it matches the Finding format.
fn parse_finding_line(line: &str) -> Option<Finding> {
    // Format: "{rule_id:<6} {SEVERITY}[\t{msg}]" optionally with " L{line}"
    // e.g. "CM-01  FAIL\tcommit title not conventional..."
    let parts: Vec<&str> = line.splitn(2, '\t').collect();
    if parts.len() != 2 {
        return None;
    }
    let prefix = parts[0].trim();
    let msg = parts[1];
    // split_whitespace collapses the double-space from {:<6} padding
    let tokens: Vec<&str> = prefix.split_whitespace().collect();
    if tokens.len() < 2 {
        return None;
    }
    let rule_id = tokens[0];
    let severity = match tokens[1] {
        "FAIL" => Severity::Fail,
        "WARN" => Severity::Warn,
        "INFO" => Severity::Info,
        _ => return None,
    };
    let line_hint = if tokens.len() == 3 {
        tokens[2]
            .strip_prefix("L")
            .and_then(|s| s.parse::<u32>().ok())
    } else {
        None
    };
    let f = Finding::new(rule_id, severity, msg);
    if let Some(lh) = line_hint {
        Some(f.with_line(lh))
    } else {
        Some(f)
    }
}

// ---------------------------------------------------------------------------
// Main entry
// ---------------------------------------------------------------------------

/// `gate pre-commit` — runs commit-title checks + dispatched topics.
pub fn run() -> i32 {
    let githooks_root =
        git::find_githooks_dir().unwrap_or_else(|| std::path::PathBuf::from(".githooks"));
    let spec_dir = githooks_root.join("spec");
    let dispatch_path = spec_dir.join("dispatch.yaml");
    let cfg = crate::shared::load_yaml(dispatch_path.to_str().unwrap_or("")).ok();

    let mut findings = Vec::new();

    // CM-01 / CM-02
    if let Some(title) = git::read_commit_editmsg() {
        let title = title.lines().next().unwrap_or("").trim();
        findings.extend(check_commit_title(title));
    }

    // CM-03
    findings.extend(check_commit_pr_consistency());

    // Dispatched topics from YAML
    let topics: Vec<String> = match &cfg {
        Some(c) => c
            .get("pre-commit")
            .and_then(|v| v.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|t| t.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        None => vec!["workspace".into(), "code".into()],
    };

    for topic in &topics {
        eprintln!("--- {} ---", topic);
        let topic_findings = match topic.as_str() {
            "workspace" => {
                let mut f = run_python_topic("workspace/tree_hygiene.py");
                f.extend(run_python_topic("workspace/file_placement.py"));
                f
            }
            "code" => run_python_topic("code/lint.py"),
            other => {
                eprintln!("unknown pre-commit topic: {}", other);
                vec![]
            }
        };
        findings.extend(topic_findings);
    }

    print_findings(&findings);
    exit_code(&findings)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cm01_rejects_non_conventional() {
        let f = check_commit_title("just a random message");
        assert_eq!(f.len(), 1);
        assert!(f[0].rule_id == "CM-01");
    }

    #[test]
    fn cm01_accepts_conventional() {
        assert!(check_commit_title("feat: add thing").is_empty());
    }

    #[test]
    fn cm01_accepts_scoped_conventional() {
        assert!(check_commit_title("fix(api): correct response").is_empty());
    }

    #[test]
    fn cm02_rejects_cjk_in_title() {
        let f = check_commit_title("feat: 添加功能");
        assert_eq!(f.len(), 1);
        assert!(f[0].rule_id == "CM-02");
    }

    #[test]
    fn cm01_skips_merge_revert() {
        assert!(check_commit_title("Merge branch 'main'").is_empty());
        assert!(check_commit_title("Revert \"feat: thing\"").is_empty());
        assert!(check_commit_title("fixup!").is_empty());
        assert!(check_commit_title("squash!").is_empty());
        assert!(check_commit_title("").is_empty());
    }

    #[test]
    fn parse_finding_line_basic() {
        let f = parse_finding_line("CM-01  FAIL\tcommit title bad");
        assert!(f.is_some());
        let f = f.unwrap();
        assert_eq!(f.rule_id, "CM-01");
        assert!(f.severity == Severity::Fail);
        assert_eq!(f.msg, "commit title bad");
    }

    #[test]
    fn parse_finding_line_with_line_hint() {
        let f = parse_finding_line("WS-02  WARN L42\tsingle-file dir");
        assert!(f.is_some());
        let f = f.unwrap();
        assert_eq!(f.rule_id, "WS-02");
        assert!(f.severity == Severity::Warn);
        assert_eq!(f.line_hint, Some(42));
    }

    #[test]
    fn parse_finding_line_rejects_garbage() {
        assert!(parse_finding_line("not a finding").is_none());
        assert!(parse_finding_line("CM-01 FAIL").is_none()); // no tab
        assert!(parse_finding_line("CM-01\tBADS\tmsg").is_none()); // bad severity
    }
}
