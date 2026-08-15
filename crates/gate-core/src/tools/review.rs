//! gate review — CRG structural analysis + ocr AI review.
//!
//! Port of `.githooks/dev/ocr_review.py`. Runs `code-review-graph detect-changes`
//! for structural analysis and `ocr review` for AI code review, then optionally
//! posts results to GitHub PR conversation / inline review.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::shared::{gh_api, run_external};
use crate::tools::git;

// ---------------------------------------------------------------------------
// OcrComment struct
// ---------------------------------------------------------------------------

/// Structured comment extracted from ocr JSON output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrComment {
    pub path: String,
    pub start_line: u64,
    pub severity: String,
    pub category: String,
    pub content: String,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// `gate review [owner/repo] [--post] [--post-inline] [--pr N]`
///
/// Runs CRG + ocr, prints to terminal, optionally posts to PR.
/// Review results are informational — never blocks merge.
pub fn run(args: &[String]) -> i32 {
    let mut positional = Vec::new();
    let mut post = false;
    let mut post_inline = false;
    let mut pr_num: Option<u64> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--post" => post = true,
            "--post-inline" => post_inline = true,
            "--pr" => {
                if i + 1 < args.len() {
                    pr_num = args[i + 1].parse().ok();
                    i += 1;
                }
            }
            _ if !args[i].starts_with("--") => positional.push(args[i].clone()),
            _ => {}
        }
        i += 1;
    }

    let repo = positional
        .first()
        .cloned()
        .or_else(|| git::derive_repo())
        .unwrap_or_default();

    let branch = git::current_branch().unwrap_or_default();
    let pr = pr_num.or_else(|| git::find_pr_for_branch(&repo, &branch).map(|n| n as u64));

    println!("=== 审查: {repo} ({branch}) ===\n");

    // 1. CRG structural analysis
    println!("--- CRG 变更影响分析 ---");
    let crg_out = run_crg();
    if crg_out.is_empty() {
        println!("（无 CRG 输出）");
    } else if crg_out.len() > 2000 {
        println!("{}", crate::shared::truncate_utf8(&crg_out, 2000));
    } else {
        println!("{crg_out}");
    }
    println!();

    // 2. ocr AI review
    println!("--- ocr AI 审查 ---");
    println!("（正在运行，LLM 可能需要几十秒...）");
    let ocr_raw = run_ocr();
    let ocr_text = format_ocr_results(&ocr_raw);
    println!("{ocr_text}");
    println!();

    // 3. Summary + post
    println!("=== 审查完成 ===");

    if (post || post_inline) && pr.is_some() && !repo.is_empty() {
        let pr = pr.unwrap();
        let comments = parse_ocr_comments(&ocr_raw);
        let has_findings = ocr_has_findings(&ocr_raw);
        let has_inline = ocr_has_inline_findings(&ocr_raw);

        if !has_findings {
            println!("无审查发现，不留言到 PR");
        } else if post_inline && has_inline {
            let inline_comments: Vec<&OcrComment> =
                comments.iter().filter(|c| c.start_line > 0).collect();
            post_inline_review(&repo, pr, &inline_comments);
            println!("已提交 inline review 到 PR #{pr}");
            if post {
                let body = review_report_body(&crg_out, &ocr_text);
                post_pr_comment(&repo, pr, &body);
                println!("已发布审查报告到 PR #{pr}");
            }
        } else if post {
            let body = review_report_body(&crg_out, &ocr_text);
            post_pr_comment(&repo, pr, &body);
            println!("已发布审查报告到 PR #{pr}");
        } else {
            // has_findings 但 (post_inline && !has_inline) 或两者都关 → 至少告知用户。
            println!(
                "有审查发现但未留言：post_inline={post_inline}（含行号 findings={has_inline}）, post={post}"
            );
        }
    }

    0
}

/// 组装 PR 评论正文：CRG 变更影响 + ocr 审查发现。
fn review_report_body(crg_out: &str, ocr_text: &str) -> String {
    format!(
        "## 审查报告\n\n### CRG 变更影响\n\n```\n{}\n```\n\n### ocr 审查发现\n\n{}",
        if crg_out.len() > 1200 { crate::shared::truncate_utf8(crg_out, 1200) } else { crg_out },
        ocr_text
    )
}

// ---------------------------------------------------------------------------
// CRG + OCR runners
// ---------------------------------------------------------------------------

/// Run `code-review-graph detect-changes --brief --base main`, return stdout.
pub fn run_crg() -> String {
    match run_external(
        &[
            "code-review-graph",
            "detect-changes",
            "--brief",
            "--base",
            "main",
        ],
        None,
    ) {
        Ok((rc, out)) if rc == 0 => out,
        Ok((_rc, out)) => format!("[CRG] {out}"),
        Err(e) => format!("[CRG] error: {e}"),
    }
}

/// Run `ocr review --format json --audience agent`, return stdout.
///
/// Uses a 600s timeout (matching the Python `_run_ocr`). If the binary is
/// missing, returns an error string instead of panicking.
pub fn run_ocr() -> String {
    use std::sync::mpsc;
    let (tx, rx) = mpsc::channel();
    let cmd_args: Vec<String> =
        ["ocr", "review", "--format", "json", "--audience", "agent"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    std::thread::spawn(move || {
        let refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
        let result = std::process::Command::new(refs[0])
            .args(&refs[1..])
            .output();
        let _ = tx.send(result);
    });
    match rx.recv_timeout(std::time::Duration::from_secs(600)) {
        Ok(Ok(o)) => {
            let stdout = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !stdout.is_empty() {
                stdout
            } else {
                let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
                if stderr.is_empty() {
                    String::new()
                } else {
                    format!("[ocr] {stderr}")
                }
            }
        }
        Ok(Err(e)) => format!("[ocr] error: {e}"),
        Err(_) => "[ocr] 超时（LLM 响应慢）".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Formatting / parsing
// ---------------------------------------------------------------------------

/// Parse ocr JSON output into readable text.
///
/// Mirrors Python `_format_ocr_results`: groups by path, formats
/// `{category}/{severity} L<line> content`. Falls back to raw text
/// truncated to 2000 chars on parse failure.
pub fn format_ocr_results(raw: &str) -> String {
    if raw.is_empty() {
        return "无结果".to_string();
    }
    match serde_json::from_str::<JsonValue>(raw) {
        Ok(data) => {
            // Primary format: {"comments": [...]}
            if let Some(comments) = data.get("comments").and_then(|c| c.as_array()) {
                if comments.is_empty() {
                    return "无审查发现".to_string();
                }
                return format_comment_list(comments);
            }
            // Legacy bare-array format
            if let Some(arr) = data.as_array() {
                if arr.is_empty() {
                    return "无审查发现".to_string();
                }
                return format_bare_ocr(arr);
            }
            "无审查发现".to_string()
        }
        Err(_) => raw[..raw.len().min(2000)].to_string(),
    }
}

/// Format the `{"comments": [...]}` JSON shape — mirrors Python `_format_ocr_results`.
fn format_comment_list(comments: &[JsonValue]) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut cur_path: Option<String> = None;
    for c in comments {
        let path = c
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string();
        if Some(&path) != cur_path.as_ref() {
            lines.push(format!("\n## {path}"));
            cur_path = Some(path);
        }
        let sev = c
            .get("severity")
            .and_then(|v| v.as_str())
            .unwrap_or("info");
        let cat = c
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let content = c
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let start = c.get("start_line").and_then(|v| v.as_u64()).unwrap_or(0);
        let loc = if start > 0 {
            format!(" L{start}")
        } else {
            String::new()
        };
        lines.push(format!("- [{cat}/{sev}]{loc} {content}"));
    }
    if lines.is_empty() {
        "无审查发现".to_string()
    } else {
        lines.join("\n")
    }
}

/// Format bare-array ocr output (legacy format from ocr_review.py).
fn format_bare_ocr(arr: &[JsonValue]) -> String {
    let mut lines = Vec::new();
    for item in arr {
        let severity = item
            .get("severity")
            .and_then(|s| s.as_str())
            .unwrap_or("unknown");
        let file = item
            .get("file")
            .and_then(|f| f.as_str())
            .unwrap_or("?");
        let line = item
            .get("line")
            .and_then(|l| l.as_u64())
            .unwrap_or(0);
        let msg = item
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("");
        lines.push(format!("- [{severity}] {file}:{line} {msg}"));
    }
    lines.join("\n")
}

/// Extract structured comments from ocr JSON output.
///
/// Returns `data["comments"]` parsed into `OcrComment` structs.
pub fn parse_ocr_comments(raw: &str) -> Vec<OcrComment> {
    match serde_json::from_str::<JsonValue>(raw) {
        Ok(data) => {
            let Some(comments) = data.get("comments").and_then(|c| c.as_array()) else {
                return Vec::new();
            };
            comments
                .iter()
                .map(|c| OcrComment {
                    path: c
                        .get("path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    start_line: c.get("start_line").and_then(|v| v.as_u64()).unwrap_or(0),
                    severity: c
                        .get("severity")
                        .and_then(|v| v.as_str())
                        .unwrap_or("info")
                        .to_string(),
                    category: c
                        .get("category")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    content: c
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                })
                .collect()
        }
        Err(_) => Vec::new(),
    }
}

/// True if `raw` ocr output contains at least one finding.
/// Mirrors `format_ocr_results`: checks `{"comments": [...]}` and bare-array
/// shapes. Non-JSON raw text (non-empty after trim) counts as findings.
pub fn ocr_has_findings(raw: &str) -> bool {
    if raw.trim().is_empty() {
        return false;
    }
    match serde_json::from_str::<JsonValue>(raw) {
        Ok(data) => {
            if let Some(comments) = data.get("comments").and_then(|c| c.as_array()) {
                return !comments.is_empty();
            }
            if let Some(arr) = data.as_array() {
                return !arr.is_empty();
            }
            false
        }
        Err(_) => true, // 非空非 JSON 的原文视为有 findings（顶部已排除空输入）
    }
}

/// True if any parsed ocr comment has a positive `start_line` (inline finding).
pub fn ocr_has_inline_findings(raw: &str) -> bool {
    parse_ocr_comments(raw).iter().any(|c| c.start_line > 0)
}

// ---------------------------------------------------------------------------
// Posting to GitHub
// ---------------------------------------------------------------------------

/// Post a PR conversation comment via `gh api`.
pub fn post_pr_comment(repo: &str, pr_num: u64, body: &str) {
    let mut params = BTreeMap::new();
    params.insert("body", body);
    let path = format!("repos/{repo}/issues/{pr_num}/comments");
    match gh_api(&path, Some(&params)) {
        Ok(_) => println!("✓ 评论已发布到 PR #{pr_num}"),
        Err(e) => eprintln!("留言失败: {e}"),
    }
}

/// Post inline review comments on the PR diff (Files changed page).
///
/// POST to `repos/{repo}/pulls/{pr}/reviews` with event=COMMENT.
pub fn post_inline_review(repo: &str, pr_num: u64, comments: &[&OcrComment]) {
    if comments.is_empty() {
        return;
    }
    let mut review_comments: Vec<JsonValue> = Vec::new();
    for c in comments {
        let line = if c.start_line > 0 { c.start_line } else { 1 };
        review_comments.push(serde_json::json!({
            "path": c.path,
            "line": line,
            "body": format!("Agent 🤖 - [{}/{}] {}", c.category, c.severity, c.content),
        }));
    }
    let payload = serde_json::json!({
        "event": "COMMENT",
        "body": "Agent 🤖 - CRG + ocr 自动审查",
        "comments": review_comments,
    });
    let path = format!("repos/{repo}/pulls/{pr_num}/reviews");
    let mut params = BTreeMap::new();
    let payload_str = payload.to_string();
    params.insert("body", payload_str.as_str());
    match gh_api(&path, Some(&params)) {
        Ok(_) => println!(
            "已提交 {} 条 inline review 到 PR #{pr_num}",
            comments.len()
        ),
        Err(e) => eprintln!("inline review 失败: {e}"),
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_ocr_results_empty() {
        assert_eq!(format_ocr_results(""), "无结果");
    }

    #[test]
    fn format_ocr_no_comments_key() {
        assert_eq!(format_ocr_results(r#"{"status": "ok"}"#), "无审查发现");
    }

    #[test]
    fn format_ocr_empty_comments() {
        assert_eq!(format_ocr_results(r#"{"comments": []}"#), "无审查发现");
    }

    #[test]
    fn format_ocr_single_comment() {
        let raw = r#"{"comments": [{"path": "src/lib.rs", "start_line": 10, "severity": "warn", "category": "style", "content": "unused var"}]}"#;
        let result = format_ocr_results(raw);
        assert!(result.contains("## src/lib.rs"));
        assert!(result.contains("[style/warn] L10 unused var"));
    }

    #[test]
    fn format_ocr_groups_by_path() {
        let raw = r#"{"comments": [
            {"path": "a.rs", "start_line": 1, "severity": "warn", "category": "style", "content": "x"},
            {"path": "a.rs", "start_line": 5, "severity": "fail", "category": "bug", "content": "y"},
            {"path": "b.rs", "start_line": 3, "severity": "info", "category": "nit", "content": "z"}
        ]}"#;
        let result = format_ocr_results(raw);
        assert!(result.contains("## a.rs"));
        assert!(result.contains("## b.rs"));
        assert_eq!(
            result.lines().filter(|l| l.starts_with("- ")).count(),
            3
        );
    }

    #[test]
    fn format_ocr_no_start_line() {
        let raw = r#"{"comments": [{"path": "z.rs", "start_line": 0, "severity": "info", "category": "", "content": "hi"}]}"#;
        let result = format_ocr_results(raw);
        assert!(result.contains("- [/info] hi"));
        assert!(!result.contains("L0"));
    }

    #[test]
    fn format_ocr_invalid_json() {
        assert_eq!(format_ocr_results("not json"), "not json");
    }

    #[test]
    fn format_ocr_bare_array_legacy() {
        let json = r#"[{"severity":"HIGH","file":"src/main.rs","line":42,"message":"bad code"}]"#;
        let text = format_ocr_results(json);
        assert!(text.contains("HIGH"));
        assert!(text.contains("src/main.rs"));
        assert!(text.contains("bad code"));
    }

    #[test]
    fn parse_ocr_comments_basic() {
        let raw = r#"{"comments": [
            {"path": "a.rs", "start_line": 10, "severity": "warn", "category": "style", "content": "x"},
            {"path": "b.rs", "start_line": 0, "severity": "info", "category": "", "content": "y"}
        ]}"#;
        let comments = parse_ocr_comments(raw);
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].path, "a.rs");
        assert_eq!(comments[0].start_line, 10);
        assert_eq!(comments[0].severity, "warn");
        assert_eq!(comments[1].path, "b.rs");
    }

    #[test]
    fn parse_ocr_comments_no_key() {
        assert!(parse_ocr_comments(r#"{"foo": 1}"#).is_empty());
    }

    #[test]
    fn parse_ocr_comments_empty() {
        assert!(parse_ocr_comments(r#"{"comments": []}"#).is_empty());
    }

    #[test]
    fn parse_ocr_comments_missing_fields() {
        let comments = parse_ocr_comments(r#"{"comments": [{}]}"#);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].path, "");
        assert_eq!(comments[0].severity, "info");
        assert_eq!(comments[0].start_line, 0);
    }

    #[test]
    fn ocr_has_findings_empty_raw() {
        assert!(!ocr_has_findings(""));
        assert!(!ocr_has_findings("   "));
    }

    #[test]
    fn ocr_has_findings_empty_comments() {
        assert!(!ocr_has_findings(r#"{"comments": []}"#));
        assert!(!ocr_has_findings(r#"{"status": "ok"}"#));
    }

    #[test]
    fn ocr_has_findings_empty_bare_array() {
        assert!(!ocr_has_findings("[]"));
    }

    #[test]
    fn ocr_has_findings_with_comments() {
        let raw = r#"{"comments": [{"path": "a.rs", "start_line": 10, "severity": "warn", "category": "style", "content": "unused var"}]}"#;
        assert!(ocr_has_findings(raw));
    }

    #[test]
    fn ocr_has_findings_bare_array_legacy() {
        assert!(ocr_has_findings(r#"[{"severity":"HIGH","file":"x.rs","line":1,"message":"bad"}]"#));
    }

    #[test]
    fn ocr_has_findings_non_json_text() {
        // Non-JSON non-empty text → treated as findings (raw fallback)
        assert!(ocr_has_findings("not json but has content"));
    }

    #[test]
    fn ocr_has_inline_findings_no_line_numbers() {
        let raw = r#"{"comments": [{"path": "a.rs", "start_line": 0, "severity": "info", "category": "", "content": "general note"}]}"#;
        assert!(!ocr_has_inline_findings(raw));
    }

    #[test]
    fn ocr_has_inline_findings_with_line_numbers() {
        let raw = r#"{"comments": [
            {"path": "a.rs", "start_line": 10, "severity": "warn", "category": "style", "content": "unused var"},
            {"path": "b.rs", "start_line": 0, "severity": "info", "category": "", "content": "no line"}
        ]}"#;
        assert!(ocr_has_inline_findings(raw));
    }

    #[test]
    fn ocr_has_inline_findings_empty() {
        assert!(!ocr_has_inline_findings(""));
        assert!(!ocr_has_inline_findings(r#"{"comments": []}"#));
    }

    #[test]
    #[ignore = "requires code-review-graph binary"]
    fn run_crg_does_not_panic() {
        let _ = run_crg();
    }

    #[test]
    #[ignore = "requires ocr binary and may invoke LLM (600s timeout)"]
    fn run_ocr_does_not_panic() {
        let _ = run_ocr();
    }

    #[test]
    fn post_pr_comment_no_panic() {
        post_pr_comment("invalid/repo", 999, "test");
    }

    #[test]
    fn post_inline_review_empty_is_noop() {
        let none: [&OcrComment; 0] = [];
        post_inline_review("invalid/repo", 1, &none);
    }

    #[test]
    fn post_inline_review_no_panic() {
        let comments = vec![OcrComment {
            path: "test.rs".to_string(),
            start_line: 1,
            severity: "info".to_string(),
            category: "nit".to_string(),
            content: "test".to_string(),
        }];
        let refs: Vec<&OcrComment> = comments.iter().collect();
        post_inline_review("invalid/repo", 999, &refs);
    }
}
