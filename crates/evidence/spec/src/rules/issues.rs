//! IS-* issue validation rules.
//!
//! Pure content checks driven by `spec/github_issues.yaml`. The `cfg`
//! parameter was deliberately dropped
//! (see sub-issue #188): the YAML-driven defaults are embedded as `const`
//! arrays below so the function works without a spec file on disk, exactly
//! like the sibling `pull_requests` rules fall back to `DEFAULT_*` when no
//! config is passed.
//!
//! API-only checks (I-18 native sub-issues, I-20 repo labels, creation-time
//! grace suggestion) live in `run`, not `check_content`, and are out of scope
//! for this pure port.

use regex::Regex;
use serde_yaml::Value as YamlValue;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::LazyLock;

use crate::shared::{Finding, Severity};

// ---------------------------------------------------------------------------
// Default config values (mirror github_issues.yaml so we work without a file)
// ---------------------------------------------------------------------------

/// `required_headings` from the YAML — sub-mode template structure (IS-01).
const DEFAULT_REQUIRED_HEADINGS: &[&str] = &[
    "Goal",
    "Background",
    "Done when",
    "Suspected areas",
    "Out of scope",
    "How to observe success",
];

/// `heading_names.done_when` — the heading `check_content` extracts for the
/// IS-04 / IS-15 checkbox checks.
const DEFAULT_DONE_WHEN_HEADING: &str = "Done when";
/// `heading_names.suspected_areas` — IS-02 non-empty check.
const DEFAULT_SUSPECTED_AREAS_HEADING: &str = "Suspected areas";

/// `title_forbidden_prefixes` — IS-00 title prefix block (case-insensitive).
const DEFAULT_TITLE_FORBIDDEN_PREFIXES: &[&str] = &["父", "issue:", "sub:", "parent:"];

/// `forbidden_brackets_in_title` — fullwidth brackets banned in titles (IS-16).
const DEFAULT_FORBIDDEN_BRACKETS: &[char] = &[
    '（', '）', '「', '」', '【', '】', '『', '』', '《', '》', '〈', '〉',
];

/// `forbidden_keywords` — body keyword block (IS-16).
const DEFAULT_FORBIDDEN_KEYWORDS: &[&str] = &["TODO", "TBD", "FIXME", "XXX"];

/// Type label set for IS-14 (mirrors the Python hard-coded set).
const TYPE_LABELS: &[&str] = &[
    "bug",
    "enhancement",
    "feature",
    "documentation",
    "chore",
    "refactor",
    "tests",
    "epic",
];

/// `keyword_label_suggestions` — flat array of (keyword, suggested-label).
/// Order does not matter for output (Python sorts+dedupes the missing set).
const DEFAULT_KEYWORD_SUGGESTIONS: &[(&str, &str)] = &[
    ("bug", "bug"),
    ("bug 报告", "bug"),
    ("重构", "refactor"),
    ("重构(", "refactor"),
    ("测试", "tests"),
    ("测试代码", "tests"),
    ("文档", "documentation"),
    ("文档清洁", "documentation"),
    ("feature", "enhancement"),
    ("新功能", "enhancement"),
    ("清理", "chore"),
    ("cleanup", "chore"),
    ("epic", "epic"),
];

// ---------------------------------------------------------------------------
// Regexes — compiled once via std::sync::LazyLock (no once_cell dependency).
// ---------------------------------------------------------------------------

fn cjk_re() -> &'static Regex {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\u4e00-\u9fff]").unwrap());
    &RE
}

/// H1 = a line starting with exactly `# ` then a non-`#`. `^# [^#]` m.
fn h1_re() -> &'static Regex {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^# [^#]").unwrap());
    &RE
}

fn heading_re() -> &'static Regex {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^#{1,6} ").unwrap());
    &RE
}

fn checkbox_re() -> &'static Regex {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*-\s*\[([ xX])\]").unwrap());
    &RE
}

fn table_re() -> &'static Regex {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\|[- ]+\|").unwrap());
    &RE
}

/// IS-09 sub-mode forbidden cross-references.  Catches all text-placeholder
/// linkage variants: `Parent:`, `Parent PR:`, `Parent #`, `Related:`,
/// `Related #`, `Depends on:`, `Blocks:`, `依赖:`.  The GitHub addSubIssue
/// mutation is the ONLY acceptable linkage — body text is a fake link.
fn cross_ref_re() -> &'static Regex {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?m)^(Depends on\s*[:：]|\*\*Depends.*[:：]|Blocks\s*[:：]|依赖[:：]|Related\s*[#：:]|Parent(?:\s+PR)?\s*[#：:])")
            .unwrap()
    });
    &RE
}

/// IS-10 sub-mode PR placeholders. Python compiles this *without* MULTILINE,
/// but the alternation has no `^` anchor so flags don't change the matches.
fn pr_placeholder_re() -> &'static Regex {
    static RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(待补\s*PR|TODO.*PR|需\s*PR|PR 关联[：:])").unwrap());
    &RE
}

fn backtick_path_re() -> &'static Regex {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`([^`\n]+)`").unwrap());
    &RE
}

const REPO_PATH_PREFIXES: &[&str] = &[
    "./",
    "../",
    ".github/",
    ".githooks/",
    "bin/",
    "crates/",
    "src/",
    "tests/",
];

fn looks_like_path_ref(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && !s.starts_with('/')
        && !s.starts_with("http://")
        && !s.starts_with("https://")
        && !s.starts_with('~')
        && !s.contains("://")
        && !s.starts_with('#')
        && !s.starts_with('-')
        && !s.contains(char::is_whitespace)
        && !s.contains(['(', ')', '*'])
        && !s.contains("::")
        && (REPO_PATH_PREFIXES
            .iter()
            .any(|prefix| s.starts_with(prefix))
            || [
                ".rs", ".py", ".sh", ".yaml", ".yml", ".toml", ".json", ".md", ".lock",
            ]
            .iter()
            .any(|suffix| s.ends_with(suffix)))
}

fn strip_line_suffix(s: &str) -> &str {
    let Some((path, suffix)) = s.rsplit_once(':') else {
        return s;
    };
    if !path.is_empty() && suffix.chars().all(|c| c.is_ascii_digit() || c == '-') {
        path
    } else {
        s
    }
}

fn path_exists_from_cwd_or_repo_root(s: &str) -> bool {
    let path = Path::new(s);
    if path.exists() || path.is_absolute() {
        return path.exists();
    }
    let Ok(cwd) = std::env::current_dir() else {
        return false;
    };
    cwd.ancestors().any(|dir| {
        (dir.join(".git").exists() || dir.join(".githooks").exists()) && dir.join(path).exists()
    })
}

fn missing_backtick_paths(body: &str) -> Vec<String> {
    let mut missing = BTreeSet::new();
    for cap in backtick_path_re().captures_iter(body) {
        let candidate = strip_line_suffix(cap[1].trim().trim_end_matches([':', ',', ';', '.']));
        if looks_like_path_ref(candidate) && !path_exists_from_cwd_or_repo_root(candidate) {
            missing.insert(candidate.to_string());
        }
    }
    missing.into_iter().collect()
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

/// Body content of the `Done when` section. Used by the GT-04/GT-05 close
/// gates: only checkboxes in this section gate closure, never the whole
/// body (Implementation Order progress boxes must not block).
pub fn done_when_section(body: &str) -> String {
    section(body, DEFAULT_DONE_WHEN_HEADING)
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

/// Collect every checkbox capture (x / X / space) under `done_when`.
fn checkbox_marks(done: &str) -> Vec<char> {
    checkbox_re()
        .captures_iter(done)
        .filter_map(|c| c.get(1).map(|m| m.as_str().chars().next().unwrap()))
        .collect()
}

// ---------------------------------------------------------------------------
// Main entry — the `check_content` port
// ---------------------------------------------------------------------------

/// Pure content validation for a GitHub issue / sub-issue. No API calls.
///
/// * `title`  — issue title (CJK required by repo convention)
/// * `body`   — issue body (markdown)
/// * `labels` — label names on the issue
/// * `mode`   — `"sub"` (default in Python) or `"parent"`
/// * `state`  — `"open"` or `"closed"`
///
/// Returns a `Vec<Finding>` in the same order as the Python `check_content`.
pub fn check_content(
    title: &str,
    body: &str,
    labels: &[&str],
    mode: &str,
    state: &str,
    cfg: Option<&YamlValue>,
) -> Vec<Finding> {
    let mut findings: Vec<Finding> = Vec::new();

    // -----------------------------------------------------------------------
    // IS-16: garbled content (literal \n / \r / U+FFFD)
    // -----------------------------------------------------------------------
    if body.contains(r"\n") || body.contains(r"\r") {
        findings.push(Finding::new(
            "IS-16",
            Severity::Fail,
            "正文含字面 \\n/\\r，应为真实换行符（用 heredoc 而非 --body 传多行）",
        ));
    }
    if body.contains('\u{fffd}') {
        findings.push(Finding::new(
            "IS-16",
            Severity::Fail,
            "正文含 U+FFFD 替换符，编码错误",
        ));
    }
    if title.contains(r"\n") || title.contains(r"\r") {
        findings.push(Finding::new("IS-16", Severity::Fail, "标题含字面 \\n/\\r"));
    }

    // -----------------------------------------------------------------------
    // IS-00: title forbidden prefixes (case-insensitive)
    // -----------------------------------------------------------------------
    let title_lower = title.to_lowercase();
    for p in DEFAULT_TITLE_FORBIDDEN_PREFIXES {
        if title_lower.starts_with(&p.to_lowercase()) {
            findings.push(Finding::new(
                "IS-00",
                Severity::Fail,
                &format!("标题禁用前缀 '{p}'，关系用 label 表达"),
            ));
        }
    }

    // IS-00: labels section forbidden in the body
    if body.contains("## Labels") {
        findings.push(Finding::new(
            "IS-00",
            Severity::Fail,
            "正文禁止 Labels 段，用 gh label 操作",
        ));
    }

    // -----------------------------------------------------------------------
    // IS-01: required template headings (sub mode only; parent n/a)
    // -----------------------------------------------------------------------
    if mode == "parent" {
        findings.push(Finding::new(
            "IS-01",
            Severity::Info,
            "parent mode: template structure n/a (Implementation Order instead)",
        ));
    } else {
        let body_hs = headings(body);
        let body_h: BTreeSet<&str> = body_hs.iter().map(|s| s.as_str()).collect();
        let missing: Vec<&&str> = DEFAULT_REQUIRED_HEADINGS
            .iter()
            .filter(|h| !body_h.contains(**h))
            .collect();
        if !missing.is_empty() {
            let joined: Vec<String> = missing.iter().map(|h| h.to_string()).collect();
            findings.push(Finding::new(
                "IS-01",
                Severity::Fail,
                &format!("missing required template headings: {}", joined.join(", ")),
            ));
        } else {
            findings.push(Finding::new(
                "IS-01",
                Severity::Info,
                "all template headings present",
            ));
        }
    }

    // -----------------------------------------------------------------------
    // IS-03: body focus — multiple H1 titles (sub mode, WARN)
    // -----------------------------------------------------------------------
    let n_h1 = h1_re().find_iter(body).count();
    if mode != "parent" && n_h1 > 1 {
        findings.push(Finding::new(
            "IS-03",
            Severity::Warn,
            &format!("multiple H1 titles ({n_h1}); body should focus one outcome"),
        ));
    } else {
        findings.push(Finding::new(
            "IS-03",
            Severity::Info,
            "body focused (or parent mode)",
        ));
    }

    // -----------------------------------------------------------------------
    // IS-04: Done when checkbox requirement (sub mode; parent n/a)
    // -----------------------------------------------------------------------
    if mode == "parent" {
        findings.push(Finding::new(
            "IS-04",
            Severity::Info,
            "parent mode: Done when n/a",
        ));
    } else {
        let done = section(body, DEFAULT_DONE_WHEN_HEADING);
        let boxes = checkbox_re().captures_iter(&done).count();
        if boxes > 0 {
            findings.push(Finding::new(
                "IS-04",
                Severity::Info,
                "Done when uses checkboxes",
            ));
        } else {
            findings.push(Finding::new(
                "IS-04",
                Severity::Fail,
                "Done when section lacks checkbox items",
            ));
        }
        if table_re().is_match(&done) {
            findings.push(Finding::new(
                "IS-04",
                Severity::Fail,
                "Done when uses a table (checkboxes required)",
            ));
        } else {
            findings.push(Finding::new(
                "IS-04",
                Severity::Info,
                "Done when has no table",
            ));
        }
    }

    // -----------------------------------------------------------------------
    // IS-02b: Suspected areas non-empty (sub mode, WARN)
    // -----------------------------------------------------------------------
    if mode != "parent" {
        let suspected = section(body, DEFAULT_SUSPECTED_AREAS_HEADING);
        if suspected.trim().is_empty() {
            findings.push(Finding::new(
                "IS-02",
                Severity::Warn,
                "Suspected areas empty; describe affected files/modules and what is not touched",
            ));
        } else {
            findings.push(Finding::new(
                "IS-02",
                Severity::Info,
                "Suspected areas populated",
            ));
        }
    }

    // -----------------------------------------------------------------------
    // IS-05 / IS-06 / IS-07: language checks
    // -----------------------------------------------------------------------
    if has_cjk(title) {
        findings.push(Finding::new("IS-05", Severity::Info, "title is Chinese"));
    } else {
        findings.push(Finding::new(
            "IS-05",
            Severity::Fail,
            "title lacks Chinese (repo convention)",
        ));
    }
    let bad_h: Vec<String> = headings(body).into_iter().filter(|h| has_cjk(h)).collect();
    if !bad_h.is_empty() {
        findings.push(Finding::new(
            "IS-06",
            Severity::Fail,
            &format!(
                "headings contain CJK (headings must be English): [{}]",
                bad_h.join(", ")
            ),
        ));
    } else {
        findings.push(Finding::new(
            "IS-06",
            Severity::Info,
            "headings are English only",
        ));
    }
    if has_cjk(body) {
        findings.push(Finding::new(
            "IS-07",
            Severity::Info,
            "body prose is Chinese",
        ));
    } else {
        findings.push(Finding::new(
            "IS-07",
            Severity::Fail,
            "body lacks Chinese prose",
        ));
    }

    // IS-08: backtick path references should exist in the repo (WARN only)
    let missing_paths = missing_backtick_paths(body);
    if missing_paths.is_empty() {
        findings.push(Finding::new(
            "IS-08",
            Severity::Info,
            "backtick path references exist or are not repo paths",
        ));
    } else {
        findings.push(Finding::new(
            "IS-08",
            Severity::Warn,
            &format!(
                "some backtick path references not found in repo: {}",
                missing_paths.join(" ")
            ),
        ));
    }

    // -----------------------------------------------------------------------
    // IS-16: forbidden keywords in body
    // -----------------------------------------------------------------------
    for kw in DEFAULT_FORBIDDEN_KEYWORDS {
        if body.contains(kw) {
            findings.push(Finding::new(
                "IS-16",
                Severity::Fail,
                &format!("body contains forbidden keyword: {kw}"),
            ));
        }
    }

    // IS-16: fullwidth brackets in title
    let brackets: BTreeSet<char> = title
        .chars()
        .filter(|c| DEFAULT_FORBIDDEN_BRACKETS.contains(c))
        .collect();
    if !brackets.is_empty() {
        let joined: Vec<String> = brackets.iter().map(|c| c.to_string()).collect();
        findings.push(Finding::new(
            "IS-16",
            Severity::Fail,
            &format!(
                "title contains fullwidth brackets: {{{}}}",
                joined.join(", ")
            ),
        ));
    } else {
        findings.push(Finding::new(
            "IS-16",
            Severity::Info,
            "no fullwidth brackets in title",
        ));
    }

    // -----------------------------------------------------------------------
    // IS-09 / IS-10 (sub mode)  |  IS-11 (parent mode)
    // -----------------------------------------------------------------------
    if mode == "sub" {
        let cross: Vec<String> = cross_ref_re()
            .find_iter(body)
            .map(|m| m.as_str().to_string())
            .collect();
        if !cross.is_empty() {
            findings.push(Finding::new(
                "IS-09",
                Severity::Fail,
                &format!("forbidden cross-references: {:?}", cross),
            ));
        } else {
            findings.push(Finding::new(
                "IS-09",
                Severity::Info,
                "no parent/dep/sibling references",
            ));
        }
        let pr_placeholder: Vec<String> = pr_placeholder_re()
            .find_iter(body)
            .map(|m| m.as_str().to_string())
            .collect();
        if !pr_placeholder.is_empty() {
            findings.push(Finding::new(
                "IS-10",
                Severity::Fail,
                &format!(
                    "sub-issue has PR placeholders/declarations: {:?}",
                    pr_placeholder
                ),
            ));
        } else {
            findings.push(Finding::new("IS-10", Severity::Info, "no PR placeholders"));
        }
    } else if mode == "parent" {
        // IS-11: parent must NOT have Done when
        if body.contains("## Done when") {
            findings.push(Finding::new(
                "IS-11",
                Severity::Fail,
                "parent must NOT have Done when section",
            ));
        } else {
            findings.push(Finding::new(
                "IS-11",
                Severity::Info,
                "parent has no Done when",
            ));
        }
    }

    // -----------------------------------------------------------------------
    // IS-14: type label present + keyword suggestions
    // -----------------------------------------------------------------------
    let labels_lower: Vec<String> = labels.iter().map(|l| l.to_lowercase()).collect();
    if labels_lower
        .iter()
        .any(|l| TYPE_LABELS.contains(&l.as_str()))
    {
        findings.push(Finding::new("IS-14", Severity::Info, "type label present"));
    } else {
        findings.push(Finding::new(
            "IS-14",
            Severity::Warn,
            "no type label (expected one of the type set)",
        ));
    }
    // keyword suggestions — case-sensitive label match (Python: suggested_label not in labels)
    let kw_map = DEFAULT_KEYWORD_SUGGESTIONS;
    let haystack = format!("{title}\n{body}").to_lowercase();
    let mut missing: Vec<String> = Vec::new();
    for (keyword, suggested) in kw_map {
        if haystack.contains(&keyword.to_lowercase()) && !labels.contains(suggested) {
            missing.push(suggested.to_string());
        }
    }
    if !missing.is_empty() {
        let mut deduped: Vec<String> = {
            let s: BTreeSet<&str> = missing.iter().map(|s| s.as_str()).collect();
            s.into_iter().map(String::from).collect()
        };
        deduped.sort();
        findings.push(Finding::new(
            "IS-14",
            Severity::Warn,
            &format!(
                "based on content keywords, consider also labeling: {}",
                deduped.join(" ")
            ),
        ));
    } else {
        findings.push(Finding::new(
            "IS-14",
            Severity::Info,
            "content keywords align with assigned labels",
        ));
    }

    // -----------------------------------------------------------------------
    // IS-15: closure rule (state=closed → sub Done when must be all checked)
    // -----------------------------------------------------------------------
    if state == "closed" {
        findings.push(Finding::new(
            "IS-15",
            Severity::Info,
            "issue closed with explicit closed event",
        ));
        if mode == "sub" {
            let done = section(body, DEFAULT_DONE_WHEN_HEADING);
            let boxes = checkbox_marks(&done);
            let total = boxes.len();
            let checked = boxes.iter().filter(|c| **c == 'x' || **c == 'X').count();
            if total == 0 {
                findings.push(Finding::new(
                    "IS-15",
                    Severity::Warn,
                    "sub-issue closed but has no Done when boxes",
                ));
            } else if total == checked {
                findings.push(Finding::new(
                    "IS-15",
                    Severity::Info,
                    &format!("sub-issue Done when all checked on close ({checked}/{total})"),
                ));
            } else {
                findings.push(Finding::new(
                    "IS-15",
                    Severity::Fail,
                    &format!(
                        "sub-issue closed with Done when unchecked ({checked}/{total}) — must tick all boxes before close"
                    ),
                ));
            }
        }
    } else {
        findings.push(Finding::new(
            "IS-15",
            Severity::Info,
            "issue open; closure rule n/a",
        ));
    }

    // Apply user-provided severity overrides (e.g. IS-14 demoted to INFO).
    crate::shared::apply_severity_overrides(&mut findings, cfg);

    findings
}

// ===========================================================================
// Tests
// ===========================================================================
