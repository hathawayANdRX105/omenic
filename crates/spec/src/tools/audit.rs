//! gate audit — validate issue/PR checkboxes and rule compliance.
//!
//! Uses the Rust issue/PR rules and the gh API client.
//!
//! Usage: `gate audit [owner/repo] [--issues=N,M] [--recent=N] [--limit=N] [--workers=N]`

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value as JsonValue;

use crate::shared::gh_api;
use crate::shared::{Finding, Severity};
use crate::tools::git;

// ---------------------------------------------------------------------------
// Regex helpers
// ---------------------------------------------------------------------------

static UNTICKED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*-\s*\[\s\]\s*(.+)").unwrap());

static CHECKBOX_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*-\s*\[([ xX])\]").unwrap());

static NEXT_HEADING_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^## ").unwrap());

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// `gate audit [owner/repo] [...]` — scan issues/PRs for rule violations.
pub fn run(args: &[String]) -> i32 {
    let mut positional = Vec::new();
    let mut specific: Option<Vec<u32>> = None;
    let mut recent: Option<u32> = None;
    let mut limit: u32 = 0;
    let mut workers: u32 = 5;

    for arg in args {
        if let Some(v) = arg.strip_prefix("--issues=") {
            specific = Some(v.split(',').filter_map(|s| s.trim().parse().ok()).collect());
        } else if let Some(v) = arg.strip_prefix("--recent=") {
            recent = v.trim().parse().ok();
        } else if let Some(v) = arg.strip_prefix("--limit=") {
            limit = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = arg.strip_prefix("--workers=") {
            workers = v.trim().parse().unwrap_or(5);
        } else if !arg.starts_with("--") {
            positional.push(arg.clone());
        }
    }

    let repo = positional.first().cloned().unwrap_or_else(derive_repo);
    if repo.is_empty() {
        eprintln!(
            "Usage: gate audit [owner/repo] [--issues=N,M] [--recent=N] [--limit=N] [--workers=N]"
        );
        return 1;
    }
    let repo = &repo;

    if let Some(days) = recent {
        return scan_recent(repo, days, limit, workers);
    }

    let nums =
        match specific {
            Some(n) => n,
            None => {
                let mut nums = Vec::new();
                if let Ok(json) = gh_api(
                    &format!(
                        "search/issues?q=repo:{}+is:closed&sort=updated&per_page=20",
                        repo
                    ),
                    None,
                ) && let Some(items) = json.get("items").and_then(|i| i.as_array())
                {
                    nums.extend(items.iter().filter_map(|i| {
                        i.get("number").and_then(|n| n.as_u64()).map(|n| n as u32)
                    }));
                }
                nums
            }
        };

    println!("检查 {} 的 issue/PR...\n", repo);
    let mut has_fail = false;

    for &num in &nums {
        let path = format!("repos/{}/issues/{}", repo, num);
        let data = match gh_api(&path, None) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let is_pr = data.get("pull_request").is_some();
        let title = data.get("title").and_then(|t| t.as_str()).unwrap_or("");
        let body = data.get("body").and_then(|b| b.as_str()).unwrap_or("");
        let state = data.get("state").and_then(|s| s.as_str()).unwrap_or("open");
        let labels: Vec<&str> = data
            .get("labels")
            .and_then(|l| l.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|l| l.get("name").and_then(|n| n.as_str()))
                    .collect()
            })
            .unwrap_or_default();

        let findings = if is_pr {
            let pr_data =
                gh_api(&format!("repos/{}/pulls/{}", repo, num), None).unwrap_or(JsonValue::Null);
            let head = pr_data
                .get("head")
                .and_then(|h| h.get("ref"))
                .and_then(|r| r.as_str())
                .unwrap_or("");
            let draft = pr_data
                .get("draft")
                .and_then(|d| d.as_bool())
                .unwrap_or(false);
            crate::rules::pull_requests::check_content(
                title, body, &labels, head, state, draft, None,
            )
        } else {
            crate::rules::issues::check_content(title, body, &labels, "sub", state, None)
        };

        let fails: Vec<&Finding> = findings
            .iter()
            .filter(|f| f.severity == Severity::Fail)
            .collect();

        if !fails.is_empty() {
            has_fail = true;
            let kind = if is_pr { "PR" } else { "issue" };
            println!("{} #{}:", kind, num);
            for f in &fails {
                println!("  {}", f.format());
            }
        }
    }

    println!("\n检查完成。");
    if has_fail { 1 } else { 0 }
}

// ---------------------------------------------------------------------------
// Required pub API
// ---------------------------------------------------------------------------

/// Derive `owner/repo` from `git remote get-url origin`. Re-exported from `git`.
pub fn derive_repo() -> String {
    git::derive_repo().unwrap_or_default()
}

/// Return the current git branch name. Re-exported from `git`.
pub fn current_branch() -> String {
    git::current_branch().unwrap_or_default()
}

/// Find the open PR number for `branch` in `repo`, or `None`. Re-exported from `git`.
pub fn find_pr(repo: &str, branch: &str) -> Option<u64> {
    git::find_pr_for_branch(repo, branch).map(|n| n as u64)
}

/// Scan recent issues/PRs created within `days` days.
///
/// `limit=0` means unlimited. `workers` is accepted for API compatibility
/// but processing is sequential (gh api rate-limit is the bottleneck).
/// Returns exit code: 0=pass, 1=has failures, 2=search error.
pub fn scan_recent(repo: &str, days: u32, limit: u32, workers: u32) -> i32 {
    let _ = workers; // sequential; rate-limit bound

    let since_date = chrono_like_date(days);
    let q = format!(
        "search/issues?q=repo:{}+created:>={}&per_page=100",
        repo, since_date
    );

    let json = match gh_api(&q, None) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("search failed: {}", e);
            return 2;
        }
    };

    let items = json
        .get("items")
        .and_then(|i| i.as_array())
        .cloned()
        .unwrap_or_default();

    let items: Vec<&JsonValue> = if limit > 0 {
        items.iter().take(limit as usize).collect()
    } else {
        items.iter().collect()
    };

    println!("最近 {} 天创建的条目: {} 个\n", days, items.len());

    let mut has_fail = false;
    for item in &items {
        let num = item.get("number").and_then(|n| n.as_u64()).unwrap_or(0) as u32;
        let is_pr = item.get("pull_request").is_some();
        let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("");
        let body = item.get("body").and_then(|b| b.as_str()).unwrap_or("");
        let state = item.get("state").and_then(|s| s.as_str()).unwrap_or("open");
        let labels: Vec<&str> = item
            .get("labels")
            .and_then(|l| l.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|l| l.get("name").and_then(|n| n.as_str()))
                    .collect()
            })
            .unwrap_or_default();

        let findings = if is_pr {
            let pr_data =
                gh_api(&format!("repos/{}/pulls/{}", repo, num), None).unwrap_or(JsonValue::Null);
            let head = pr_data
                .get("head")
                .and_then(|h| h.get("ref"))
                .and_then(|r| r.as_str())
                .unwrap_or("");
            let draft = pr_data
                .get("draft")
                .and_then(|d| d.as_bool())
                .unwrap_or(false);
            crate::rules::pull_requests::check_content(
                title, body, &labels, head, state, draft, None,
            )
        } else {
            crate::rules::issues::check_content(title, body, &labels, "sub", state, None)
        };

        let fails: Vec<&Finding> = findings
            .iter()
            .filter(|f| f.severity == Severity::Fail)
            .collect();

        if !fails.is_empty() {
            has_fail = true;
            let kind = if is_pr { "PR" } else { "issue" };
            println!("{} #{}:", kind, num);
            for f in &fails {
                println!("  {}", f.format());
            }
            println!();
        }
    }

    if has_fail { 1 } else { 0 }
}

/// Check an issue's "Done when" section for unchecked items.
///
/// Returns list of unticked item descriptions.
/// Mirrors Python `check_issue_done_when`.
pub fn check_issue_done_when(repo: &str, num: u64) -> Vec<String> {
    let body = fetch_issue_body(repo, num);
    if body.is_empty() {
        return Vec::new();
    }
    let sec = section(&body, "Done when");
    UNTICKED_RE
        .captures_iter(&sec)
        .map(|c| {
            c.get(1)
                .map(|m| m.as_str().trim().to_string())
                .unwrap_or_default()
        })
        .collect()
}

/// Check a PR's body for Construction/Checklist checkbox counts and Fixes linkage.
///
/// Returns list of problem descriptions.
/// Mirrors Python `check_pr_body`.
pub fn check_pr_body(repo: &str, num: u64) -> Vec<String> {
    let body = fetch_pr_body(repo, num);
    if body.is_empty() {
        return Vec::new();
    }
    let mut problems = Vec::new();

    for sec_name in ["Construction plan", "Checklist"] {
        let sec = section(&body, sec_name);
        let boxes: Vec<_> = CHECKBOX_RE.captures_iter(&sec).collect();
        if boxes.len() < 2 {
            problems.push(format!(
                "{} 只有 {} 个 checkbox，需要至少 2 个",
                sec_name,
                boxes.len()
            ));
        }
        let unticked: Vec<String> = UNTICKED_RE
            .captures_iter(&sec)
            .map(|c| {
                c.get(1)
                    .map(|m| m.as_str().trim().to_string())
                    .unwrap_or_default()
            })
            .collect();
        if !unticked.is_empty() {
            problems.push(format!("{sec_name} 未勾: {unticked:?}"));
        }
    }

    if !body.contains("Fixes") && !body.contains("Closes") && !body.contains("Resolves") {
        problems.push("无 Fixes 关联".to_string());
    }

    problems
}

// ---------------------------------------------------------------------------
// Section extraction
// ---------------------------------------------------------------------------

/// Extract the body text under `## <heading>` up to the next `## ` heading.
fn section(body: &str, heading: &str) -> String {
    let pattern = format!(r"(?m)^## {}\s*$", regex_escape(heading));
    let Ok(re) = Regex::new(&pattern) else {
        return String::new();
    };
    let Some(m) = re.find(body) else {
        return String::new();
    };
    let rest = &body[m.end()..];
    match NEXT_HEADING_RE.find(rest) {
        Some(n) => rest[..n.start()].to_string(),
        None => rest.to_string(),
    }
}

/// Escape regex special characters in a literal string.
fn regex_escape(s: &str) -> String {
    s.chars()
        .map(|c| {
            if r"\.+*?()|[]{}^$".contains(c) {
                format!("\\{c}")
            } else {
                c.to_string()
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// GitHub fetch helpers
// ---------------------------------------------------------------------------

fn fetch_issue_body(repo: &str, num: u64) -> String {
    let path = format!("repos/{repo}/issues/{num}");
    gh_api(&path, None)
        .ok()
        .and_then(|v| v.get("body").and_then(|b| b.as_str()).map(String::from))
        .unwrap_or_default()
}

fn fetch_pr_body(repo: &str, num: u64) -> String {
    let path = format!("repos/{repo}/pulls/{num}");
    gh_api(&path, None)
        .ok()
        .and_then(|v| v.get("body").and_then(|b| b.as_str()).map(String::from))
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Date helper (no chrono dependency)
// ---------------------------------------------------------------------------

/// Compute date N days ago as YYYY-MM-DD without pulling in chrono.
fn chrono_like_date(days_ago: u32) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let day_secs = 86_400u64;
    let target = secs.saturating_sub(u64::from(days_ago) * day_secs);
    unix_to_ymd(target)
}

/// Convert a Unix timestamp (seconds) to `YYYY-MM-DD` (UTC).
fn unix_to_ymd(ts: u64) -> String {
    let days = (ts / 86_400) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_to_ymd_known_dates() {
        assert_eq!(unix_to_ymd(1704067200), "2024-01-01");
        assert_eq!(unix_to_ymd(1703980800), "2023-12-31");
        assert_eq!(unix_to_ymd(1709164800), "2024-02-29");
    }

    #[test]
    fn section_extracts_text() {
        let body = "intro\n## Done when\n- [ ] item one\n- [x] item two\n\n## Other\nstuff";
        let sec = section(body, "Done when");
        assert!(sec.contains("item one"));
        assert!(sec.contains("item two"));
        assert!(!sec.contains("Other"));
    }

    #[test]
    fn section_missing_returns_empty() {
        assert_eq!(section("no heading here", "Done when"), "");
    }

    #[test]
    fn section_last_heading_takes_rest() {
        let body = "## Done when\n- [ ] a";
        assert!(section(body, "Done when").contains("- [ ] a"));
    }

    #[test]
    fn regex_escape_special_chars() {
        assert_eq!(regex_escape("Done when"), "Done when");
        assert_eq!(regex_escape("a.b"), "a\\.b");
        assert_eq!(regex_escape("(test)"), "\\(test\\)");
    }

    #[test]
    fn check_issue_done_when_parsing() {
        let body = "## Done when\n- [ ] unchecked item\n- [x] checked item";
        let sec = section(body, "Done when");
        let unticked: Vec<String> = UNTICKED_RE
            .captures_iter(&sec)
            .map(|c| c.get(1).unwrap().as_str().trim().to_string())
            .collect();
        assert_eq!(unticked, vec!["unchecked item"]);
    }

    #[test]
    fn check_pr_body_checkbox_count() {
        let body = "## Construction plan\n- [ ] one\n## Checklist\n- [ ] a\n- [ ] b\n\nFixes #1";
        let sec_plan = section(body, "Construction plan");
        assert_eq!(CHECKBOX_RE.captures_iter(&sec_plan).count(), 1);

        let sec_check = section(body, "Checklist");
        assert_eq!(CHECKBOX_RE.captures_iter(&sec_check).count(), 2);

        assert!(body.contains("Fixes"));
    }

    #[test]
    fn check_pr_body_no_fixes_link() {
        let body = "## Construction plan\n- [ ] one\n- [ ] two";
        assert!(!body.contains("Fixes"));
        assert!(!body.contains("Closes"));
        assert!(!body.contains("Resolves"));
    }

    #[test]
    fn extract_fixes_from_body() {
        let re = regex::Regex::new(r"(?:Fixes|Closes|Resolves)\s+#(\d+)").unwrap();
        let body = "Fixes #42 and Closes #99";
        let fixes: Vec<String> = re
            .captures_iter(body)
            .map(|c| c.get(1).unwrap().as_str().to_string())
            .collect();
        assert_eq!(fixes, vec!["42", "99"]);
    }

    #[test]
    fn derive_repo_does_not_panic() {
        let _ = derive_repo();
    }

    #[test]
    fn current_branch_does_not_panic() {
        let _ = current_branch();
    }

    #[test]
    fn find_pr_does_not_panic() {
        let _ = find_pr("nonexistent/nonexistent-repo-xyz", "nonexistent-branch");
    }

    #[test]
    fn scan_recent_does_not_panic() {
        let _ = scan_recent("nonexistent/nonexistent-repo-xyz", 1, 0, 1);
    }
}
