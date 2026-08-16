//! IS-* issue validation rules — port of `.githooks/github/issues.py` `check_content`.
//!
//! Pure content checks driven by `spec/github_issues.yaml`.  Mirrors the
//! Python logic rule-for-rule.  The `cfg` parameter was deliberately dropped
//! (see sub-issue #188): the YAML-driven defaults are embedded as `const`
//! arrays below so the function works without a spec file on disk, exactly
//! like the sibling `pull_requests` rules fall back to `DEFAULT_*` when no
//! config is passed.
//!
//! API-only checks (I-18 native sub-issues, I-20 repo labels, creation-time
//! grace suggestion) live in `run`, not `check_content`, and are out of scope
//! for this pure port.

use regex::Regex;
use std::collections::BTreeSet;
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
    // IS-09 / IS-10 (sub mode)  |  IS-11 / IS-13 (parent mode)
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
        // IS-13: Implementation Order optional (INFO either way)
        if body.contains("## Implementation Order") {
            findings.push(Finding::new(
                "IS-13",
                Severity::Info,
                "Implementation Order present (optional)",
            ));
        } else {
            findings.push(Finding::new(
                "IS-13",
                Severity::Info,
                "no Implementation Order section (optional)",
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
            Severity::Fail,
            "no type label (expected one of the type set)",
        ));
    }
    // keyword suggestions — case-sensitive label match (Python: suggested_label not in labels)
    let kw_map = DEFAULT_KEYWORD_SUGGESTIONS;
    let haystack = format!("{title}\n{body}").to_lowercase();
    let mut missing: Vec<String> = Vec::new();
    for (keyword, suggested) in kw_map {
        if haystack.contains(&keyword.to_lowercase()) && !labels.iter().any(|l| *l == *suggested) {
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

    findings
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::Severity;

    /// A realistic sub-issue body that satisfies every IS-* structural rule:
    /// all required headings present, Done when has checkboxes (no table),
    /// Suspected areas populated, headings English, Chinese prose, no
    /// forbidden keywords, no fullwidth brackets, no cross-refs, no PR
    /// placeholders.
    const GOOD_SUB_BODY: &str = "\
## Goal
实现新的功能。

## Background
需要这个功能来支持更多场景。

## Done when
- [ ] 第一步完成
- [ ] 第二步完成

## Suspected areas
- src/a.rs: 入口修改
- src/b.rs: 辅助函数

## Out of scope
不修改文档。

## How to observe success
运行测试通过。
";

    /// A realistic parent-issue body: no Done when, optional Implementation Order.
    const GOOD_PARENT_BODY: &str = "\
## Goal
跟踪子任务进度。

## Implementation Order
- (#101)
- (#102)
";

    /// Helper: collect findings matching a rule_id.
    fn find_rule<'a>(findings: &'a [Finding], rule: &str) -> Vec<&'a Finding> {
        findings.iter().filter(|f| f.rule_id == rule).collect()
    }

    // -----------------------------------------------------------------------
    // IS-00: title forbidden prefixes + Labels section forbidden
    // -----------------------------------------------------------------------

    #[test]
    fn is00_sub_prefix_forbidden_fails() {
        let f = check_content(
            "sub: 添加功能",
            GOOD_SUB_BODY,
            &["enhancement"],
            "sub",
            "open",
        );
        let r = find_rule(&f, "IS-00");
        assert!(
            r.iter()
                .any(|x| x.severity == Severity::Fail && x.msg.contains("禁用前缀"))
        );
    }

    #[test]
    fn is00_case_insensitive_prefix() {
        let f = check_content(
            "ISSUE: 添加功能",
            GOOD_SUB_BODY,
            &["enhancement"],
            "sub",
            "open",
        );
        assert!(
            find_rule(&f, "IS-00")
                .iter()
                .any(|x| x.severity == Severity::Fail)
        );
    }

    #[test]
    fn is00_labels_section_forbidden() {
        let body = format!("{GOOD_SUB_BODY}## Labels\n- enhancement\n");
        let f = check_content("添加功能", &body, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-00")
                .iter()
                .any(|x| x.severity == Severity::Fail && x.msg.contains("禁止 Labels")),
            "body with ## Labels must FAIL IS-00"
        );
    }

    #[test]
    fn is00_clean_title_no_prefix_fail() {
        let f = check_content("添加新功能", GOOD_SUB_BODY, &["enhancement"], "sub", "open");
        assert!(
            !find_rule(&f, "IS-00")
                .iter()
                .any(|x| x.severity == Severity::Fail)
        );
    }

    // -----------------------------------------------------------------------
    // IS-01: required headings (sub mode FAIL on missing; parent INFO)
    // -----------------------------------------------------------------------

    #[test]
    fn is01_sub_all_headings_info() {
        let f = check_content("添加功能", GOOD_SUB_BODY, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-01")
                .iter()
                .any(|x| x.severity == Severity::Info && x.msg.contains("all template headings"))
        );
    }

    #[test]
    fn is01_sub_missing_heading_fails() {
        let bad = GOOD_SUB_BODY.replace("## Out of scope\n不修改文档。\n", "");
        let f = check_content("添加功能", &bad, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-01")
                .iter()
                .any(|x| x.severity == Severity::Fail && x.msg.contains("Out of scope"))
        );
    }

    #[test]
    fn is01_parent_mode_info_na() {
        let f = check_content("epic 跟踪", GOOD_PARENT_BODY, &["epic"], "parent", "open");
        assert!(
            find_rule(&f, "IS-01")
                .iter()
                .all(|x| x.severity == Severity::Info)
        );
    }

    // -----------------------------------------------------------------------
    // IS-02: Suspected areas non-empty (sub mode, WARN)
    // -----------------------------------------------------------------------

    #[test]
    fn is02_sub_populated_info() {
        let f = check_content("添加功能", GOOD_SUB_BODY, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-02")
                .iter()
                .any(|x| x.severity == Severity::Info)
        );
    }

    #[test]
    fn is02_sub_empty_warns() {
        let bad = GOOD_SUB_BODY.replace(
            "## Suspected areas\n- src/a.rs: 入口修改\n- src/b.rs: 辅助函数\n",
            "## Suspected areas\n\n",
        );
        let f = check_content("添加功能", &bad, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-02")
                .iter()
                .any(|x| x.severity == Severity::Warn && x.msg.contains("Suspected areas empty"))
        );
    }

    // -----------------------------------------------------------------------
    // IS-03: body focus — multiple H1 (sub mode, WARN)
    // -----------------------------------------------------------------------

    #[test]
    fn is03_sub_single_h1_ok() {
        let f = check_content("添加功能", GOOD_SUB_BODY, &["enhancement"], "sub", "open");
        let r = find_rule(&f, "IS-03");
        assert!(r.iter().any(|x| x.severity == Severity::Info));
        assert!(!r.iter().any(|x| x.severity == Severity::Warn));
    }

    #[test]
    fn is03_sub_multiple_h1_warns() {
        let bad = format!("# 标题一\n\n{GOOD_SUB_BODY}# 标题二\n");
        let f = check_content("添加功能", &bad, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-03")
                .iter()
                .any(|x| x.severity == Severity::Warn && x.msg.contains("multiple H1"))
        );
    }

    // -----------------------------------------------------------------------
    // IS-04: Done when checkbox requirement (sub mode; parent n/a)
    // -----------------------------------------------------------------------

    #[test]
    fn is04_sub_checkboxes_info() {
        let f = check_content("添加功能", GOOD_SUB_BODY, &["enhancement"], "sub", "open");
        let r = find_rule(&f, "IS-04");
        assert!(
            r.iter()
                .any(|x| x.severity == Severity::Info && x.msg.contains("uses checkboxes"))
        );
        assert!(!r.iter().any(|x| x.severity == Severity::Fail));
    }

    #[test]
    fn is04_sub_no_checkbox_fails() {
        let bad = GOOD_SUB_BODY.replace(
            "## Done when\n- [ ] 第一步完成\n- [ ] 第二步完成\n",
            "## Done when\nplain prose no boxes\n",
        );
        let f = check_content("添加功能", &bad, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-04")
                .iter()
                .any(|x| x.severity == Severity::Fail && x.msg.contains("lacks checkbox"))
        );
    }

    #[test]
    fn is04_sub_table_fails() {
        let bad = GOOD_SUB_BODY.replace(
            "## Done when\n- [ ] 第一步完成\n- [ ] 第二步完成\n",
            "## Done when\n| a | b |\n|---|---|\n| 1 | 2 |\n",
        );
        let f = check_content("添加功能", &bad, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-04")
                .iter()
                .any(|x| x.severity == Severity::Fail && x.msg.contains("table"))
        );
    }

    #[test]
    fn is04_parent_na_info() {
        let f = check_content("epic 跟踪", GOOD_PARENT_BODY, &["epic"], "parent", "open");
        assert!(
            find_rule(&f, "IS-04")
                .iter()
                .all(|x| x.severity == Severity::Info)
        );
    }

    // -----------------------------------------------------------------------
    // IS-05: title Chinese (CJK check)
    // -----------------------------------------------------------------------

    #[test]
    fn is05_chinese_title_info() {
        let f = check_content("添加新功能", GOOD_SUB_BODY, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-05")
                .iter()
                .any(|x| x.severity == Severity::Info)
        );
    }

    #[test]
    fn is05_english_only_title_fails() {
        let f = check_content(
            "add new feature",
            GOOD_SUB_BODY,
            &["enhancement"],
            "sub",
            "open",
        );
        assert!(
            find_rule(&f, "IS-05")
                .iter()
                .any(|x| x.severity == Severity::Fail && x.msg.contains("lacks Chinese"))
        );
    }

    // -----------------------------------------------------------------------
    // IS-06: headings English only (no CJK in headings)
    // -----------------------------------------------------------------------

    #[test]
    fn is06_english_headings_info() {
        let f = check_content("添加功能", GOOD_SUB_BODY, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-06")
                .iter()
                .any(|x| x.severity == Severity::Info)
        );
    }

    #[test]
    fn is06_cjk_heading_fails() {
        let bad = GOOD_SUB_BODY.replace("## Goal", "## 目标");
        let f = check_content("添加功能", &bad, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-06")
                .iter()
                .any(|x| x.severity == Severity::Fail && x.msg.contains("headings contain CJK"))
        );
    }

    // -----------------------------------------------------------------------
    // IS-07: body has Chinese prose
    // -----------------------------------------------------------------------

    #[test]
    fn is07_chinese_body_info() {
        let f = check_content("添加功能", GOOD_SUB_BODY, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-07")
                .iter()
                .any(|x| x.severity == Severity::Info)
        );
    }

    #[test]
    fn is07_no_chinese_body_fails() {
        // English-only body that still has all required headings + checkboxes.
        let english_body = "\
## Goal
implement feature.

## Background
need it.

## Done when
- [ ] step one
- [ ] step two

## Suspected areas
- src/a.rs

## Out of scope
none.

## How to observe success
tests pass.
";
        let f = check_content("添加功能", english_body, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-07")
                .iter()
                .any(|x| x.severity == Severity::Fail && x.msg.contains("lacks Chinese prose"))
        );
    }

    // -----------------------------------------------------------------------
    // IS-09: sub mode forbidden cross-references
    // -----------------------------------------------------------------------

    #[test]
    fn is09_sub_no_cross_info() {
        let f = check_content("添加功能", GOOD_SUB_BODY, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-09")
                .iter()
                .any(|x| x.severity == Severity::Info)
        );
    }

    #[test]
    fn is09_sub_depends_on_fails() {
        let bad = format!("{GOOD_SUB_BODY}Depends on: #42\n");
        let f = check_content("添加功能", &bad, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-09")
                .iter()
                .any(|x| x.severity == Severity::Fail
                    && x.msg.contains("forbidden cross-references"))
        );
    }

    #[test]
    fn is09_sub_related_hash_fails() {
        let bad = format!("{GOOD_SUB_BODY}Related #7\n");
        let f = check_content("添加功能", &bad, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-09")
                .iter()
                .any(|x| x.severity == Severity::Fail
                    && x.msg.contains("forbidden cross-references"))
        );
    }

    #[test]
    fn is09_sub_parent_colon_fails() {
        let bad = format!("{GOOD_SUB_BODY}Parent: #205\n");
        let f = check_content("添加功能", &bad, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-09")
                .iter()
                .any(|x| x.severity == Severity::Fail
                    && x.msg.contains("forbidden cross-references"))
        );
    }

    #[test]
    fn is09_sub_parent_hash_fails() {
        let bad = format!("{GOOD_SUB_BODY}Parent #205\n");
        let f = check_content("添加功能", &bad, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-09")
                .iter()
                .any(|x| x.severity == Severity::Fail
                    && x.msg.contains("forbidden cross-references"))
        );
    }

    #[test]
    fn is09_sub_related_colon_fails() {
        let bad = format!("{GOOD_SUB_BODY}Related: #205\n");
        let f = check_content("添加功能", &bad, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-09")
                .iter()
                .any(|x| x.severity == Severity::Fail
                    && x.msg.contains("forbidden cross-references"))
        );
    }

    #[test]
    fn is09_sub_parent_pr_colon_fails() {
        let bad = format!("{GOOD_SUB_BODY}Parent PR: #10\n");
        let f = check_content("添加功能", &bad, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-09")
                .iter()
                .any(|x| x.severity == Severity::Fail
                    && x.msg.contains("forbidden cross-references"))
        );
    }

    #[test]
    fn is09_sub_blocks_fails() {
        let bad = format!("{GOOD_SUB_BODY}Blocks: #99\n");
        let f = check_content("添加功能", &bad, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-09")
                .iter()
                .any(|x| x.severity == Severity::Fail
                    && x.msg.contains("forbidden cross-references"))
        );
    }

    #[test]
    fn is09_sub_dependency_cn_fails() {
        let bad = format!("{GOOD_SUB_BODY}依赖: #88\n");
        let f = check_content("添加功能", &bad, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-09")
                .iter()
                .any(|x| x.severity == Severity::Fail
                    && x.msg.contains("forbidden cross-references"))
        );
    }

    // -----------------------------------------------------------------------
    // IS-10: sub mode PR placeholders forbidden
    // -----------------------------------------------------------------------

    #[test]
    fn is10_sub_no_placeholder_info() {
        let f = check_content("添加功能", GOOD_SUB_BODY, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-10")
                .iter()
                .any(|x| x.severity == Severity::Info)
        );
    }

    #[test]
    fn is10_sub_pr_placeholder_fails() {
        let bad = format!("{GOOD_SUB_BODY}待补 PR\n");
        let f = check_content("添加功能", &bad, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-10")
                .iter()
                .any(|x| x.severity == Severity::Fail && x.msg.contains("PR placeholders"))
        );
    }

    // -----------------------------------------------------------------------
    // IS-11: parent must NOT have Done when
    // -----------------------------------------------------------------------

    #[test]
    fn is11_parent_no_done_when_info() {
        let f = check_content("epic 跟踪", GOOD_PARENT_BODY, &["epic"], "parent", "open");
        assert!(
            find_rule(&f, "IS-11")
                .iter()
                .any(|x| x.severity == Severity::Info)
        );
    }

    #[test]
    fn is11_parent_has_done_when_fails() {
        let bad = format!("{GOOD_PARENT_BODY}## Done when\n- [ ] x\n");
        let f = check_content("epic 跟踪", &bad, &["epic"], "parent", "open");
        assert!(
            find_rule(&f, "IS-11")
                .iter()
                .any(|x| x.severity == Severity::Fail)
        );
    }

    // -----------------------------------------------------------------------
    // IS-13: Implementation Order optional (parent mode INFO)
    // -----------------------------------------------------------------------

    #[test]
    fn is13_parent_impl_order_info() {
        let f = check_content("epic 跟踪", GOOD_PARENT_BODY, &["epic"], "parent", "open");
        let r = find_rule(&f, "IS-13");
        assert!(!r.is_empty());
        assert!(r.iter().all(|x| x.severity == Severity::Info));
    }

    // -----------------------------------------------------------------------
    // IS-14: type label present + keyword suggestions
    // -----------------------------------------------------------------------

    #[test]
    fn is14_type_label_present_info() {
        let f = check_content("添加功能", GOOD_SUB_BODY, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-14")
                .iter()
                .any(|x| x.severity == Severity::Info && x.msg.contains("type label present"))
        );
    }

    #[test]
    fn is14_no_type_label_fails() {
        let f = check_content("添加功能", GOOD_SUB_BODY, &["question"], "sub", "open");
        assert!(
            find_rule(&f, "IS-14")
                .iter()
                .any(|x| x.severity == Severity::Fail && x.msg.contains("no type label"))
        );
    }

    #[test]
    fn is14_keyword_suggestion_warn() {
        // body contains "文档" → suggest "documentation"; "enhancement" present
        // but "documentation" is not → WARN suggestion.
        let body = GOOD_SUB_BODY.to_string() + "\n文档清理\n";
        let f = check_content("添加功能", &body, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-14")
                .iter()
                .any(|x| x.severity == Severity::Warn && x.msg.contains("consider"))
        );
    }

    #[test]
    fn is14_keywords_aligned_info() {
        // GOOD_SUB_BODY contains both "文档" and "测试" (per "运行测试通过"):
        // providing both suggested labels → aligned → INFO.
        let body = GOOD_SUB_BODY.to_string() + "\n文档\n";
        let f = check_content(
            "添加功能",
            &body,
            &["documentation", "tests"],
            "sub",
            "open",
        );
        assert!(
            find_rule(&f, "IS-14")
                .iter()
                .any(|x| x.severity == Severity::Info && x.msg.contains("keywords align"))
        );
    }

    // -----------------------------------------------------------------------
    // IS-15: closure rule — closed sub must have all Done when checked
    // -----------------------------------------------------------------------

    #[test]
    fn is15_open_info_na() {
        let f = check_content("添加功能", GOOD_SUB_BODY, &["enhancement"], "sub", "open");
        let r = find_rule(&f, "IS-15");
        assert!(
            r.iter()
                .any(|x| x.severity == Severity::Info && x.msg.contains("closure rule n/a"))
        );
    }

    #[test]
    fn is15_closed_sub_all_checked_info() {
        let body = GOOD_SUB_BODY
            .replace("- [ ] 第一步完成", "- [x] 第一步完成")
            .replace("- [ ] 第二步完成", "- [x] 第二步完成");
        let f = check_content("添加功能", &body, &["enhancement"], "sub", "closed");
        let r = find_rule(&f, "IS-15");
        assert!(
            r.iter()
                .any(|x| x.severity == Severity::Info && x.msg.contains("all checked on close"))
        );
        assert!(!r.iter().any(|x| x.severity == Severity::Fail));
    }

    #[test]
    fn is15_closed_sub_unchecked_fails() {
        let f = check_content("添加功能", GOOD_SUB_BODY, &["enhancement"], "sub", "closed");
        let r = find_rule(&f, "IS-15");
        // GOOD body has two unchecked boxes → FAIL
        assert!(
            r.iter()
                .any(|x| x.severity == Severity::Fail && x.msg.contains("Done when unchecked"))
        );
    }

    #[test]
    fn is15_closed_sub_no_boxes_warns() {
        let bad = GOOD_SUB_BODY.replace(
            "## Done when\n- [ ] 第一步完成\n- [ ] 第二步完成\n",
            "## Done when\nplain prose no boxes\n",
        );
        let f = check_content("添加功能", &bad, &["enhancement"], "sub", "closed");
        assert!(
            find_rule(&f, "IS-15")
                .iter()
                .any(|x| x.severity == Severity::Warn && x.msg.contains("no Done when boxes"))
        );
    }

    // -----------------------------------------------------------------------
    // IS-16: garbled content + forbidden keywords + fullwidth brackets
    // -----------------------------------------------------------------------

    #[test]
    fn is16_literal_backslash_n_body_fails() {
        let bad = GOOD_SUB_BODY.to_string() + r"\n literal here";
        let f = check_content("添加功能", &bad, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-16")
                .iter()
                .any(|x| x.severity == Severity::Fail && x.msg.contains("字面"))
        );
    }

    #[test]
    fn is16_replacement_char_fails() {
        let bad = GOOD_SUB_BODY.to_string() + "\u{fffd}";
        let f = check_content("添加功能", &bad, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-16")
                .iter()
                .any(|x| x.severity == Severity::Fail && x.msg.contains("U+FFFD"))
        );
    }

    #[test]
    fn is16_forbidden_keyword_todo_fails() {
        let bad = GOOD_SUB_BODY.to_string() + "\nTODO: finish\n";
        let f = check_content("添加功能", &bad, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-16")
                .iter()
                .any(|x| x.severity == Severity::Fail
                    && x.msg.contains("forbidden keyword")
                    && x.msg.contains("TODO"))
        );
    }

    #[test]
    fn is16_fullwidth_bracket_in_title_fails() {
        let f = check_content(
            "添加（功能）",
            GOOD_SUB_BODY,
            &["enhancement"],
            "sub",
            "open",
        );
        assert!(
            find_rule(&f, "IS-16")
                .iter()
                .any(|x| x.severity == Severity::Fail && x.msg.contains("fullwidth brackets"))
        );
    }

    #[test]
    fn is16_clean_title_no_brackets_info() {
        let f = check_content("添加功能", GOOD_SUB_BODY, &["enhancement"], "sub", "open");
        assert!(
            find_rule(&f, "IS-16")
                .iter()
                .any(|x| x.severity == Severity::Info && x.msg.contains("no fullwidth brackets"))
        );
    }

    // -----------------------------------------------------------------------
    // Helpers: _section / _headings / _has_cjk
    // -----------------------------------------------------------------------

    #[test]
    fn helper_section_extracts_body() {
        let s = section("## Done when\n- [ ] a\n\n## Out of scope\n", "Done when");
        assert!(s.contains("- [ ] a"));
        assert!(!s.contains("Out of scope"));
    }

    #[test]
    fn helper_section_missing_returns_empty() {
        assert_eq!(section("## Other\nx\n", "Done when"), "");
    }

    #[test]
    fn helper_headings_stripped() {
        let h = headings("## Goal\nx\n### Sub\ny\n");
        assert_eq!(h, vec!["Goal", "Sub"]);
    }

    #[test]
    fn helper_has_cjk_true_false() {
        assert!(has_cjk("中文"));
        assert!(!has_cjk("english"));
        assert!(!has_cjk(""));
    }

    // -----------------------------------------------------------------------
    // Smoke test: a complete sub-issue body produces no FAIL at all
    // -----------------------------------------------------------------------

    #[test]
    fn smoke_good_sub_body_no_fail() {
        let f = check_content("添加新功能", GOOD_SUB_BODY, &["enhancement"], "sub", "open");
        let fails: Vec<&Finding> = f.iter().filter(|x| x.severity == Severity::Fail).collect();
        assert!(
            fails.is_empty(),
            "GOOD_SUB_BODY should produce no FAIL; got: {:?}",
            fails.iter().map(|x| &x.msg).collect::<Vec<_>>()
        );
    }

    #[test]
    fn smoke_good_parent_body_no_fail() {
        let f = check_content("epic 跟踪", GOOD_PARENT_BODY, &["epic"], "parent", "open");
        let fails: Vec<&Finding> = f.iter().filter(|x| x.severity == Severity::Fail).collect();
        assert!(
            fails.is_empty(),
            "GOOD_PARENT_BODY should produce no FAIL; got: {:?}",
            fails.iter().map(|x| &x.msg).collect::<Vec<_>>()
        );
    }
}
