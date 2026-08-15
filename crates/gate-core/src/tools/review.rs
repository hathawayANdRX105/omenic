//! gate review — local review tool integrating CRG (structure analysis) + ocr (AI review).
//!
//! Port of `.githooks/dev/ocr_review.py`. Runs CRG for change detection
//! and ocr for AI-based code review. Results are informational — never
//! blocks merge.
//!
//! Usage: `gate review [owner/repo] [--post]`

use serde_json::Value as JsonValue;

use crate::shared::run_external;
use crate::tools::git;

/// `gate review [owner/repo] [--post]` — run CRG + ocr review.
pub fn run(args: &[String]) -> i32 {
    let mut positional = Vec::new();
    let mut post = false;
    for arg in args {
        match arg.as_str() {
            "--post" => post = true,
            _ if !arg.starts_with("--") => positional.push(arg.clone()),
            _ => {}
        }
    }

    let repo = positional
        .first()
        .map(|s| s.clone())
        .or_else(|| git::derive_repo())
        .unwrap_or_default();

    println!("== gate review (CRG + ocr) ==");

    // CRG: code-review-graph detect-changes
    let crg_out = run_crg();
    println!("{}", crg_out);

    // ocr: AI review
    println!("（ocr 运行中，LLM 需几分钟...）");
    let ocr_out = run_ocr();
    let ocr_text = format_ocr_results(&ocr_out);
    println!("{}", ocr_text);

    if post && !repo.is_empty() {
        let branch = git::current_branch().unwrap_or_default();
        let pr_num = git::find_pr_for_branch(&repo, &branch);
        if let Some(pr) = pr_num {
            let body = format!("## CRG Review\n\n```\n{}\n```\n\n## ocr Results\n\n{}", crg_out, ocr_text);
            post_pr_comment(&repo, pr, &body);
        } else {
            eprintln!("no open PR found for branch '{}', skipping --post", branch);
        }
    }

    // Review results are informational — never blocks
    0
}

fn run_crg() -> String {
    match run_external(&["code-review-graph", "detect-changes"], None) {
        Ok((_, out)) => out,
        Err(e) => format!("[CRG] error: {}", e),
    }
}

fn run_ocr() -> String {
    match run_external(&["ocr", "--json"], None) {
        Ok((_, out)) => out,
        Err(e) => format!("[ocr] error: {}", e),
    }
}

/// Parse ocr JSON output into readable text. Falls back to raw on parse error.
fn format_ocr_results(raw: &str) -> String {
    if raw.is_empty() {
        return "无结果".to_string();
    }
    match serde_json::from_str::<JsonValue>(raw) {
        Ok(json) => {
            let mut lines = Vec::new();
            if let Some(arr) = json.as_array() {
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
                    lines.push(format!("- [{}] {}:{} {}", severity, file, line, msg));
                }
                lines.join("\n")
            } else {
                raw[..raw.len().min(2000)].to_string()
            }
        }
        Err(_) => raw[..raw.len().min(2000)].to_string(),
    }
}

fn post_pr_comment(repo: &str, pr_num: u32, body: &str) {
    let mut params = BTreeMap::new();
    params.insert("body", body);
    let path = format!("repos/{}/issues/{}/comments", repo, pr_num);
    match crate::shared::gh_api(&path, Some(&params)) {
        Ok(_) => println!("✓ 评论已发布到 PR #{}", pr_num),
        Err(e) => eprintln!("留言失败: {}", e),
    }
}

use std::collections::BTreeMap;

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
    fn format_ocr_results_json_array() {
        let json = r#"[{"severity":"HIGH","file":"src/main.rs","line":42,"message":"bad code"}]"#;
        let text = format_ocr_results(json);
        assert!(text.contains("HIGH"));
        assert!(text.contains("src/main.rs"));
        assert!(text.contains("bad code"));
    }

    #[test]
    fn format_ocr_results_non_json_fallbacks() {
        let text = format_ocr_results("not json at all");
        assert_eq!(text, "not json at all");
    }
}
