//! merge hook — validate PR, reviews, cleanup before squash-merge.
//!
//! Port of `.githooks/hooks/merge`. Uses the already-ported Rust rules
//! (pull_requests, reviews) for GitHub validation, plus the Rust cleanup
//! module for branch cleanup.
//!
//! Usage: `gate merge <owner/repo> <pr_number> [--dry-run]`

use crate::shared::{
    Finding, Severity, apply_global_overrides, exit_code, gh_api, gh_api_paginate, load_yaml,
    print_findings,
};
use crate::tools::{checklist, cleanup, git};
/// `gate merge <owner/repo> <pr_number> [--dry-run]` — pre-merge validation.
pub fn run(args: &[String]) -> i32 {
    let mut positional = Vec::new();
    let mut dry_run = false;
    for arg in args {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            _ if !arg.starts_with("--") => positional.push(arg.clone()),
            _ => {}
        }
    }

    if positional.len() < 2 {
        eprintln!("Usage: gate merge <owner/repo> <pr_number> [--dry-run]");
        return 2;
    }

    let repo = &positional[0];
    let pr_num: u32 = match positional[1].parse() {
        Ok(n) => n,
        Err(_) => {
            eprintln!("invalid PR number: {}", positional[1]);
            return 2;
        }
    };

    let githooks =
        git::find_githooks_dir().unwrap_or_else(|| std::path::PathBuf::from(".githooks"));
    let spec_dir = githooks.join("spec");
    let dispatch_path = spec_dir.join("dispatch.yaml");
    let cfg = load_yaml(dispatch_path.to_str().unwrap_or("")).ok();

    let mut findings = Vec::new();

    println!("== Merge #{} ({}) ==", pr_num, repo);
    if dry_run {
        println!("[DRY-RUN]");
    }

    // Run topics from dispatch.yaml merge section
    let topics: Vec<String> = match &cfg {
        Some(c) => c
            .get("merge")
            .and_then(|v| v.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|t| t.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        None => vec![
            "github/pull_requests".into(),
            "github/reviews".into(),
            "cleanup".into(),
        ],
    };

    for topic in &topics {
        println!("--- {} ---", topic);
        match topic.as_str() {
            "github/pull_requests" => findings.extend(run_pr_rules(repo, pr_num)),
            "github/reviews" => findings.extend(run_review_rules(repo, pr_num, &spec_dir)),
            "cleanup" => {
                findings.extend(cleanup::run(dry_run));
                findings.extend(crate::tools::tests_check::run());
                findings.extend(crate::tools::docs_hygiene::run());
            }
            "checklist" => findings.extend(checklist::run_all(checklist::HookScope::Merge)),
            other => eprintln!("unknown merge topic: {}", other),
        }
    }

    // Local CRG + ocr review requirement
    println!("--- review (CRG + ocr) ---");
    findings.extend(check_pr_review(repo, pr_num));

    apply_global_overrides(&mut findings);
    print_findings(&findings);
    let rc = exit_code(&findings);
    if rc == 0 && !dry_run {
        // Extract Fixes from PR body
        if let Ok(pr) = gh_api(&format!("repos/{}/pulls/{}", repo, pr_num), None) {
            let body = pr.get("body").and_then(|b| b.as_str()).unwrap_or("");
            let fixes = extract_fixes(body);
            if !fixes.is_empty() {
                print!("Fixes: #{}", fixes.join(", #"));
                println!();
            }
            if let Some(head) = pr
                .get("head")
                .and_then(|h| h.get("ref"))
                .and_then(|r| r.as_str())
            {
                println!("Branch: {}", head);
            }
        }
    }

    rc
}

fn run_pr_rules(repo: &str, pr_num: u32) -> Vec<Finding> {
    match gh_api(&format!("repos/{}/pulls/{}", repo, pr_num), None) {
        Ok(pr) => crate::rules::pull_requests::check_content(
            pr.get("title").and_then(|t| t.as_str()).unwrap_or(""),
            pr.get("body").and_then(|b| b.as_str()).unwrap_or(""),
            &labels_from(&pr),
            pr.get("head")
                .and_then(|h| h.get("ref"))
                .and_then(|r| r.as_str())
                .unwrap_or(""),
            pr.get("state").and_then(|s| s.as_str()).unwrap_or("open"),
            pr.get("draft").and_then(|d| d.as_bool()).unwrap_or(false),
            None, // cfg defaults embedded
        ),
        Err(e) => vec![Finding::new(
            "PR",
            Severity::Fail,
            &format!("could not fetch PR #{}: {}", pr_num, e),
        )],
    }
}

fn run_review_rules(repo: &str, pr_num: u32, spec_dir: &std::path::Path) -> Vec<Finding> {
    let review_cfg = load_yaml(spec_dir.join("github_reviews.yaml").to_str().unwrap_or("")).ok();

    // Collect all review + issue comments
    let mut bodies = Vec::new();

    if let Ok(comments) = gh_api_paginate(&format!("repos/{}/pulls/{}/comments", repo, pr_num), 100)
    {
        for c in &comments {
            if let Some(body) = c.get("body").and_then(|b| b.as_str()) {
                bodies.push(body.to_string());
            }
        }
    }
    if let Ok(comments) = gh_api(&format!("repos/{}/issues/{}/comments", repo, pr_num), None)
        && let Some(arr) = comments.as_array()
    {
        for c in arr {
            if let Some(body) = c.get("body").and_then(|b| b.as_str()) {
                bodies.push(body.to_string());
            }
        }
    }

    match &review_cfg {
        Some(cfg) => crate::rules::reviews::run(&bodies, cfg),
        None => vec![Finding::new(
            "RV",
            Severity::Warn,
            "github_reviews.yaml not found, review checks skipped",
        )],
    }
}

fn check_pr_review(repo: &str, pr_num: u32) -> Vec<Finding> {
    let pr_files = match gh_api(&format!("repos/{}/pulls/{}/files", repo, pr_num), None) {
        Ok(f) => f,
        Err(e) => {
            return vec![Finding::new(
                "RV-07",
                Severity::Fail,
                &format!("could not fetch PR files: {}", e),
            )];
        }
    };
    let file_count = pr_files.as_array().map(|a| a.len()).unwrap_or(0);
    if file_count == 0 {
        println!("PR #{} 无文件改动，无需审查。", pr_num);
        return vec![rv07_decide(0, "", "")];
    }

    println!("PR #{} 有 {} 个文件改动，必须审查：", pr_num, file_count);
    if let Some(files) = pr_files.as_array() {
        for f in files.iter().take(10) {
            let name = f.get("filename").and_then(|n| n.as_str()).unwrap_or("?");
            let add = f.get("additions").and_then(|a| a.as_u64()).unwrap_or(0);
            let del = f.get("deletions").and_then(|d| d.as_u64()).unwrap_or(0);
            println!("  {} (+{}/-{})", name, add, del);
        }
    }

    // Run CRG + ocr — both must succeed for RV-07 to pass.
    // Uses review.rs runners which surface timeouts/errors as "[CRG] …" /
    // "[ocr] …" markers; rv07_decide classifies those into FAIL.
    println!("（CRG + ocr 运行中，LLM 需几分钟...）");
    let crg_out = crate::tools::review::run_crg();
    println!("{}", crate::shared::truncate_utf8(&crg_out, 1200));

    let ocr_out = crate::tools::review::run_ocr();
    println!("{}", crate::shared::truncate_utf8(&ocr_out, 1500));

    vec![rv07_decide(file_count, &crg_out, &ocr_out)]
}

/// Classify RV-07 from file count + CRG/ocr outputs.
///
/// - No files (0) → INFO skip ("无需审查").
/// - Both CRG + ocr succeed → INFO pass.
/// - Either CRG or ocr fails / times out → FAIL (block).
///
/// Failure markers:
///   CRG: `run_crg()` prefixes every failure with "[CRG]" (non-zero exit or
///        spawn error); success returns the raw stdout.
///   ocr: `run_ocr()` prefixes every failure with "[ocr]" (spawn error, stderr,
///        or 超时 timeout); success returns the raw stdout.
fn rv07_decide(file_count: usize, crg_out: &str, ocr_out: &str) -> Finding {
    const CRG_FAIL_PREFIX: &str = "[CRG]";
    const OCR_FAIL_PREFIX: &str = "[ocr]";
    const TRUNC_MAX: usize = 200;
    const ELLIPSIS: &str = "…";
    const TRUNC_ELLIPSIS_BYTES: usize = ELLIPSIS.len(); // 3 字节 UTF-8
    if file_count == 0 {
        return Finding::new("RV-07", Severity::Info, "无需审查（无文件改动）");
    }
    let crg_failed = crg_out.is_empty() || crg_out.starts_with(CRG_FAIL_PREFIX);
    let ocr_failed = ocr_out.is_empty() || ocr_out.starts_with(OCR_FAIL_PREFIX);
    let cap = |s: &str| {
        let s = s.trim();
        if s.len() > TRUNC_MAX {
            format!(
                "{ELLIPSIS}{}",
                crate::shared::truncate_utf8(s, TRUNC_MAX.saturating_sub(TRUNC_ELLIPSIS_BYTES))
            )
        } else {
            s.to_string()
        }
    };
    match (crg_failed, ocr_failed) {
        (true, true) => Finding::new(
            "RV-07",
            Severity::Fail,
            &format!(
                "CRG 和 ocr 均失败\ncrg: {}\nocr: {}",
                cap(crg_out),
                cap(ocr_out)
            ),
        ),
        (true, false) => Finding::new(
            "RV-07",
            Severity::Fail,
            &format!("CRG 失败: {}", cap(crg_out)),
        ),
        (false, true) => Finding::new(
            "RV-07",
            Severity::Fail,
            &format!("ocr 失败/超时: {}", cap(ocr_out)),
        ),
        (false, false) => Finding::new("RV-07", Severity::Info, "审查完成: CRG + ocr 双重审查通过"),
    }
}

fn extract_fixes(body: &str) -> Vec<String> {
    let re = regex::Regex::new(r"(?:Fixes|Closes|Resolves)\s+#(\d+)").unwrap();
    re.captures_iter(body)
        .map(|c| c.get(1).unwrap().as_str().to_string())
        .collect()
}

fn labels_from(pr: &serde_json::Value) -> Vec<&str> {
    pr.get("labels")
        .and_then(|l| l.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|l| l.get("name").and_then(|n| n.as_str()))
                .collect()
        })
        .unwrap_or_default()
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::Severity;

    #[test]
    fn ocr_timeout_is_fail() {
        let f = rv07_decide(3, "changes detected", "[ocr] 超时（LLM 响应慢）");
        assert_eq!(f.severity, Severity::Fail);
        assert_eq!(f.rule_id, "RV-07");
    }

    #[test]
    fn ocr_success_is_info() {
        let f = rv07_decide(3, "risk: low", r#"{"comments": []}"#);
        assert_eq!(f.severity, Severity::Info);
        assert!(f.msg.contains("双重审查通过"));
    }

    #[test]
    fn crg_failure_is_fail() {
        let f = rv07_decide(3, "[CRG] error: not found", r#"{"comments": []}"#);
        assert_eq!(f.severity, Severity::Fail);
    }

    #[test]
    fn both_fail_is_fail() {
        let f = rv07_decide(3, "[CRG] error: x", "[ocr] error: y");
        assert_eq!(f.severity, Severity::Fail);
    }

    #[test]
    fn crg_empty_is_fail() {
        let f = rv07_decide(3, "", r#"{"comments": []}"#);
        assert_eq!(f.severity, Severity::Fail);
    }

    #[test]
    fn no_file_changes_skips_review() {
        let f = rv07_decide(0, "", "");
        assert_eq!(f.severity, Severity::Info);
        assert!(f.msg.contains("无需审查"));
    }

    #[test]
    fn ocr_empty_is_fail() {
        let f = rv07_decide(3, "risk: low", "");
        assert_eq!(f.severity, Severity::Fail);
    }
}
