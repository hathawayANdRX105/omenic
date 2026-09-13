//! PR-* validation rules.
//!
//! Pure content checks driven by `spec/github_pull_requests.yaml`, including
//! the fork "user:" prefix strip on `head_ref` (PR-08).

use regex::Regex;
use serde_yaml::Value as YamlValue;
use std::collections::HashSet;

use crate::shared::{Finding, Severity};

// ---------------------------------------------------------------------------
// Default config values (mirror the YAML so the function works without a file)
// ---------------------------------------------------------------------------

const DEFAULT_BODY_HEADINGS: &[&str] = &[
    "Issue",
    "What",
    "Why",
    "Construction plan",
    "Delivery record",
    "How to test",
    "Checklist",
];

const DEFAULT_BRANCH_PREFIXES: &[&str] = &[
    "feat/", "fix/", "chore/", "epic/", "main", "master", "release/",
];

const DEFAULT_TYPE_LABELS: &[&str] = &[
    "bug",
    "enhancement",
    "feature",
    "documentation",
    "chore",
    "refactor",
    "tests",
    "epic",
];

// ponytail: keyword map as a flat array of (keyword, label) pairs; avoids
// allocating a HashMap when the config is absent or empty.
const DEFAULT_KEYWORD_SUGGESTIONS: &[(&str, &str)] = &[
    ("bug", "bug"),
    ("修复", "bug"),
    ("新功能", "enhancement"),
    ("功能", "enhancement"),
    ("功能添加", "enhancement"),
    ("清理", "chore"),
    ("优化", "refactor"),
    ("测试", "tests"),
    ("代码质量", "refactor"),
];

// ---------------------------------------------------------------------------
// Regexes — compiled once via std::sync::LazyLock (no once_cell dependency).
// ---------------------------------------------------------------------------

fn cjk_re() -> &'static Regex {
    static RE: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"[\u4e00-\u9fff]").unwrap());
    &RE
}

fn heading_re() -> &'static Regex {
    static RE: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"(?m)^#{1,6} ").unwrap());
    &RE
}

fn checkbox_re() -> &'static Regex {
    static RE: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"(?m)^\s*-\s*\[([ xX])\]").unwrap());
    &RE
}

fn conv_commit_re() -> &'static Regex {
    static RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r"^(feat|fix|chore|docs|style|refactor|test|ci|build|perf|revert)(\(.+\))?:\s+")
            .unwrap()
    });
    &RE
}

fn fixes_re() -> &'static Regex {
    static RE: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"(?:Fixes|Closes|Resolves)\s+#(\d+)").unwrap());
    &RE
}

fn text_links_re() -> &'static Regex {
    static RE: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"(?:Part of|Related)\s+#(\d+)").unwrap());
    &RE
}

// ---------------------------------------------------------------------------
// Helpers — direct ports of the Python _functions
// ---------------------------------------------------------------------------

/// True if `s` contains any CJK ideograph (U+4E00–U+9FFF).
fn has_cjk(s: &str) -> bool {
    cjk_re().is_match(s)
}

/// Extract the body text under a `## heading` line, up to the next `## `.
/// Mirrors Python `_section`.
fn section(body: &str, heading: &str) -> String {
    let pattern = format!(r"(?m)^## {}\s*$", regex::escape(heading));
    let re = match Regex::new(&pattern) {
        Ok(r) => r,
        Err(_) => return String::new(),
    };
    let m = match re.find(body) {
        Some(m) => m,
        None => return String::new(),
    };
    let rest = &body[m.end()..];
    let next_re = match Regex::new(r"(?m)^## ") {
        Ok(r) => r,
        Err(_) => return rest.to_string(),
    };
    match next_re.find(rest) {
        Some(n) => rest[..n.start()].to_string(),
        None => rest.to_string(),
    }
}

/// Return all heading texts (leading `#`s stripped) in line order.
/// Mirrors Python `_headings`.
fn headings(body: &str) -> Vec<String> {
    let re = heading_re();
    body.lines()
        .filter(|line| re.is_match(line))
        .map(|line| line.trim().trim_start_matches('#').trim().to_string())
        .collect()
}

/// Extract all `Fixes #N` / `Closes #N` / `Resolves #N` issue numbers.
fn extract_fixes(body: &str) -> Vec<String> {
    fixes_re()
        .captures_iter(body)
        .map(|c| c.get(1).unwrap().as_str().to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// Config extraction helpers
// ---------------------------------------------------------------------------

fn cfg_body_headings(cfg: Option<&YamlValue>) -> Vec<String> {
    if let Some(arr) = cfg
        .and_then(|c| c.get("required_body_headings"))
        .and_then(|v| v.as_sequence())
    {
        let vals: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        if !vals.is_empty() {
            return vals;
        }
    }
    DEFAULT_BODY_HEADINGS
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn cfg_branch_prefixes(cfg: Option<&YamlValue>) -> Vec<String> {
    if let Some(arr) = cfg
        .and_then(|c| c.get("allowed_branch_prefixes"))
        .and_then(|v| v.as_sequence())
    {
        let vals: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        if !vals.is_empty() {
            return vals;
        }
    }
    DEFAULT_BRANCH_PREFIXES
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn cfg_type_labels(cfg: Option<&YamlValue>) -> Vec<String> {
    if let Some(arr) = cfg
        .and_then(|c| c.get("type_labels_cfg"))
        .and_then(|v| v.as_sequence())
    {
        let vals: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        if !vals.is_empty() {
            return vals;
        }
    }
    DEFAULT_TYPE_LABELS.iter().map(|s| s.to_string()).collect()
}

/// Extract keyword→label suggestions map from config YAML.
/// Falls back to `DEFAULT_KEYWORD_SUGGESTIONS` when absent or empty.
fn cfg_keyword_suggestions(cfg: Option<&YamlValue>) -> Vec<(String, String)> {
    if let Some(map) = cfg
        .and_then(|c| c.get("keyword_label_suggestions"))
        .and_then(|v| v.as_mapping())
    {
        let pairs: Vec<(String, String)> = map
            .iter()
            .filter_map(|(k, v)| {
                let key = k.as_str()?.to_string();
                let val = v.as_str()?.to_string();
                Some((key, val))
            })
            .collect();
        if !pairs.is_empty() {
            return pairs;
        }
    }
    DEFAULT_KEYWORD_SUGGESTIONS
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

// ---------------------------------------------------------------------------
// Main entry — the `check_content` port
// ---------------------------------------------------------------------------

/// Pure content validation for a pull request.  No API calls.
///
/// * `title`     — PR title
/// * `body`      — PR body (markdown)
/// * `labels`    — label names on the PR
/// * `head_ref`  — head ref name (may include fork "user:" prefix)
/// * `state`     — "open" or "closed"/"merged"
/// * `draft`     — whether the PR is a draft
/// * `cfg`       — optional parsed YAML config (the spec file)
///
/// Returns a `Vec<Finding>` in the same order as the Python version.
pub fn check_content(
    title: &str,
    body: &str,
    labels: &[&str],
    head_ref: &str,
    state: &str,
    draft: bool,
    cfg: Option<&YamlValue>,
) -> Vec<Finding> {
    let mut findings: Vec<Finding> = Vec::new();

    // PR-01 title English / PR-02 conventional commit
    if has_cjk(title) {
        findings.push(Finding::new(
            "PR-01",
            Severity::Fail,
            "title contains CJK (title should be English)",
        ));
    } else {
        findings.push(Finding::new("PR-01", Severity::Info, "title is English"));
    }
    if conv_commit_re().is_match(title) {
        findings.push(Finding::new(
            "PR-02",
            Severity::Info,
            "conventional commit title",
        ));
    } else {
        findings.push(Finding::new(
            "PR-02",
            Severity::Warn,
            &format!(
                "title not conventional commit (repo template allows natural English): {title}"
            ),
        ));
    }

    // PR-03 body structure headings
    let body_h: HashSet<String> = headings(body).into_iter().collect();
    let required = cfg_body_headings(cfg);
    for h in &required {
        if body_h.contains(h) {
            findings.push(Finding::new(
                "PR-03",
                Severity::Info,
                &format!("heading present: {h}"),
            ));
        } else {
            findings.push(Finding::new(
                "PR-03",
                Severity::Fail,
                &format!("missing heading: ## {h}"),
            ));
        }
    }

    // PR-07 Construction plan / Checklist ≥2 checkboxes
    let cb_re = checkbox_re();
    if body_h.contains("Construction plan") {
        let plan = section(body, "Construction plan");
        let boxes = cb_re.captures_iter(&plan).count();
        if boxes < 2 {
            findings.push(Finding::new(
                "PR-07",
                Severity::Fail,
                &format!("Construction plan 必须至少 2 个 checkbox，当前 {boxes} 个"),
            ));
        }
    }
    if body_h.contains("Checklist") {
        let checklist = section(body, "Checklist");
        let boxes = cb_re.captures_iter(&checklist).count();
        if boxes < 2 {
            findings.push(Finding::new(
                "PR-07",
                Severity::Fail,
                &format!("Checklist 必须至少 2 个 checkbox，当前 {boxes} 个"),
            ));
        }
    }

    // PR-04 headings English only + What section Chinese prose
    let all_headings = headings(body);
    let bad_h: Vec<&String> = all_headings.iter().filter(|h| has_cjk(h)).collect();
    if !bad_h.is_empty() {
        let joined: Vec<String> = bad_h.iter().map(|h| h.to_string()).collect();
        findings.push(Finding::new(
            "PR-04",
            Severity::Fail,
            &format!(
                "headings contain CJK (headings must be English): [{}]",
                joined.join(", ")
            ),
        ));
    } else {
        findings.push(Finding::new(
            "PR-04",
            Severity::Info,
            "headings are English only",
        ));
    }
    let what = section(body, "What");
    if has_cjk(&what) {
        findings.push(Finding::new(
            "PR-04",
            Severity::Info,
            "What section has Chinese prose",
        ));
    } else {
        findings.push(Finding::new(
            "PR-04",
            Severity::Warn,
            "What section has no Chinese prose (template requires Chinese)",
        ));
    }

    // PR-05 issue linkage — Fixes #N checks
    let fixes = extract_fixes(body);
    // dedupe + sort numerically (Python: sorted(set(fixes), key=int))
    let mut fixes_unique: Vec<i32> = fixes
        .iter()
        .filter_map(|s| s.parse::<i32>().ok())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    fixes_unique.sort();
    let fixes_count = fixes_unique.len();

    if state == "open" && fixes_count > 0 {
        findings.push(Finding::new(
            "PR-05",
            Severity::Warn,
            "open PR already uses Fixes # (may close issue prematurely)",
        ));
    } else {
        findings.push(Finding::new(
            "PR-05",
            Severity::Info,
            "no premature Fixes while open (or PR not open)",
        ));
    }
    if fixes_count == 1 {
        findings.push(Finding::new("PR-05", Severity::Info, "exactly one Fixes #"));
    } else if fixes_count == 0 {
        if draft {
            findings.push(Finding::new(
                "PR-05",
                Severity::Info,
                "draft PR, Fixes may appear at merge authorization",
            ));
        } else {
            findings.push(Finding::new(
                "PR-05",
                Severity::Warn,
                "no Fixes # yet (needs one primary issue before merge)",
            ));
        }
    } else {
        findings.push(Finding::new(
            "PR-05",
            Severity::Warn,
            &format!("multiple Fixes # ({fixes_count}): one PR should close one issue"),
        ));
    }
    if fixes_count <= 1 {
        findings.push(Finding::new("PR-05", Severity::Info, "one primary issue"));
    } else {
        findings.push(Finding::new(
            "PR-05",
            Severity::Warn,
            "one PR should close one primary issue",
        ));
    }

    // PR-10 plain-text Part of / Related links — INFO
    let text_links: Vec<String> = text_links_re()
        .captures_iter(body)
        .map(|c| c.get(1).unwrap().as_str().to_string())
        .collect();
    if !text_links.is_empty() {
        findings.push(Finding::new(
            "PR-10",
            Severity::Info,
            &format!(
                "Part of/Related #({}) 是纯文本，不产生 GitHub 关联；epic 关联通过 Fixes 的 sub-issue 层级或 UI development 面板",
                text_links.join(", ")
            ),
        ));
    } else {
        findings.push(Finding::new(
            "PR-10",
            Severity::Info,
            "no plain-text Part of/Related links",
        ));
    }

    // PR-06 type label + keyword suggestions (appears twice in Python — once
    // before P-11 and once before P-31; both emit identical findings.  We
    // replicate the first occurrence here.)
    let type_labels = cfg_type_labels(cfg);
    let label_set: HashSet<&str> = labels.iter().copied().collect();
    if type_labels.iter().any(|l| label_set.contains(l.as_str())) {
        findings.push(Finding::new("PR-06", Severity::Info, "type label present"));
    } else {
        findings.push(Finding::new(
            "PR-06",
            Severity::Fail,
            "no type label (expected one of the type set)",
        ));
    }
    let kw_map = cfg_keyword_suggestions(cfg);
    {
        let haystack = format!("{title}\n{body}").to_lowercase();
        let mut missing: Vec<String> = Vec::new();
        for (keyword, suggested) in &kw_map {
            if haystack.contains(&keyword.to_lowercase()) && !label_set.contains(suggested.as_str())
            {
                missing.push(suggested.clone());
            }
        }
        if !missing.is_empty() {
            // dedupe + sort (Python: sorted(set(missing)))
            let deduped: Vec<String> = {
                let s: HashSet<&str> = missing.iter().map(|s| s.as_str()).collect();
                let mut v: Vec<String> = s.into_iter().map(String::from).collect();
                v.sort();
                v
            };
            findings.push(Finding::new(
                "PR-06",
                Severity::Warn,
                &format!(
                    "based on content keywords, consider also labeling: {}",
                    deduped.join(" ")
                ),
            ));
        } else {
            findings.push(Finding::new(
                "PR-06",
                Severity::Info,
                "content keywords align with assigned labels",
            ));
        }
    }

    // PR-08 branch name — strip fork "user:" prefix before prefix check.
    // This is the fix for the real bug: a forkPR's head_ref is "user:branch",
    // and we must check the *branch* part, not "user:branch".
    let allowed = cfg_branch_prefixes(cfg);
    let branch = if head_ref.contains(':') {
        // Python: head_ref.rsplit(":", 1)[-1] — last segment after the final ":"
        head_ref.rsplit(':').next().unwrap_or(head_ref)
    } else {
        head_ref
    };
    if branch.is_empty() || !allowed.iter().any(|p| branch.starts_with(p.as_str())) {
        findings.push(Finding::new(
            "PR-08",
            Severity::Fail,
            &format!(
                "branch name not allowed: {branch} (allowed prefixes: {:?})",
                allowed
            ),
        ));
    } else {
        findings.push(Finding::new(
            "PR-08",
            Severity::Info,
            &format!("branch name OK: {branch} (prefixes: {:?})", allowed),
        ));
    }

    // PR-06 duplicate block (Python emits it a second time before P-31).
    // We replicate faithfully.
    if type_labels.iter().any(|l| label_set.contains(l.as_str())) {
        findings.push(Finding::new("PR-06", Severity::Info, "type label present"));
    } else {
        findings.push(Finding::new(
            "PR-06",
            Severity::Fail,
            "no type label (expected one of the type set)",
        ));
    }
    {
        let haystack = format!("{title}\n{body}").to_lowercase();
        let mut missing: Vec<String> = Vec::new();
        for (keyword, suggested) in &kw_map {
            if haystack.contains(&keyword.to_lowercase()) && !label_set.contains(suggested.as_str())
            {
                missing.push(suggested.clone());
            }
        }
        if !missing.is_empty() {
            let deduped: Vec<String> = {
                let s: HashSet<&str> = missing.iter().map(|s| s.as_str()).collect();
                let mut v: Vec<String> = s.into_iter().map(String::from).collect();
                v.sort();
                v
            };
            findings.push(Finding::new(
                "PR-06",
                Severity::Warn,
                &format!(
                    "based on content keywords, consider also labeling: {}",
                    deduped.join(" ")
                ),
            ));
        } else {
            findings.push(Finding::new(
                "PR-06",
                Severity::Info,
                "content keywords align with assigned labels",
            ));
        }
    }
    // Apply user-provided severity overrides (e.g. PR-05 demoted to WARN).
    crate::shared::apply_severity_overrides(&mut findings, cfg);

    findings
}

// ===========================================================================
// Tests
// ===========================================================================
