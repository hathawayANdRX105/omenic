//! gate audit — validate issue/PR checkboxes and rule compliance.
//!
//! Port of `.githooks/dev/audit.py`. Uses the already-ported Rust rules
//! (issues, pull_requests) and the gh API client.
//!
//! Usage: `gate audit <owner/repo> [--issues=N,M] [--recent=N] [--limit=N]`


use serde_json::Value as JsonValue;

use crate::shared::{gh_api, Finding, Severity};

/// `gate audit <owner/repo> [...]` — scan issues/PRs for rule violations.
pub fn run(args: &[String]) -> i32 {
    let mut positional = Vec::new();
    let mut specific: Option<Vec<u32>> = None;
    let mut recent: Option<u32> = None;
    let mut limit: u32 = 0;

    for arg in args {
        if let Some(v) = arg.strip_prefix("--issues=") {
            specific = Some(v.split(',').filter_map(|s| s.trim().parse().ok()).collect());
        } else if let Some(v) = arg.strip_prefix("--recent=") {
            recent = v.trim().parse().ok();
        } else if let Some(v) = arg.strip_prefix("--limit=") {
            limit = v.trim().parse().unwrap_or(0);
        } else if !arg.starts_with("--") {
            positional.push(arg.clone());
        }
    }

    if positional.is_empty() {
        eprintln!("Usage: gate audit <owner/repo> [--issues=N,M] [--recent=N] [--limit=N]");
        return 1;
    }
    let repo = &positional[0];

    if let Some(days) = recent {
        return scan_recent(repo, days, limit);
    }

    let nums = match specific {
        Some(n) => n,
        None => {
            // Default: recent closed issues + merged PRs
            let mut nums = Vec::new();
            if let Ok(json) = gh_api(
                &format!(
                    "search/issues?q=repo:{}+is:closed&sort=updated&per_page=20",
                    repo
                ),
                None,
            ) {
                if let Some(items) = json.get("items").and_then(|i| i.as_array()) {
                    nums.extend(items.iter().filter_map(|i| {
                        i.get("number").and_then(|n| n.as_u64()).map(|n| n as u32)
                    }));
                }
            }
            nums
        }
    };

    println!("检查 {} 的 issue/PR...\n", repo);
    let mut has_fail = false;

    for &num in &nums {
        // Fetch issue/PR data
        let path = format!("repos/{}/issues/{}", repo, num);
        let data = match gh_api(&path, None) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let is_pr = data.get("pull_request").is_some();
        let title = data
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("");
        let body = data
            .get("body")
            .and_then(|b| b.as_str())
            .unwrap_or("");
        let state = data
            .get("state")
            .and_then(|s| s.as_str())
            .unwrap_or("open");
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
            // Fetch full PR data for head_ref, draft
            let pr_data = gh_api(&format!("repos/{}/pulls/{}", repo, num), None)
                .unwrap_or(JsonValue::Null);
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
            crate::rules::issues::check_content(title, body, &labels, "sub", state)
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
    if has_fail {
        1
    } else {
        0
    }
}

/// Scan recent issues/PRs created within `days` days.
fn scan_recent(repo: &str, days: u32, limit: u32) -> i32 {
    let since_date = format!(
        "{}",
        chrono_like_date(days)
    );
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

    println!(
        "最近 {} 天创建的条目: {} 个\n",
        days,
        items.len()
    );

    let mut has_fail = false;
    for item in &items {
        let num = item
            .get("number")
            .and_then(|n| n.as_u64())
            .unwrap_or(0) as u32;
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
            let pr_data = gh_api(&format!("repos/{}/pulls/{}", repo, num), None)
                .unwrap_or(JsonValue::Null);
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
            crate::rules::issues::check_content(title, body, &labels, "sub", state)
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

    let rc = if has_fail { 1 } else { 0 };
    rc
}

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
    // Algorithm: Howard Hinnant's days_from_civil in reverse.
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
    format!("{:04}-{:02}-{:02}", y, m, d)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_to_ymd_known_dates() {
        // 2024-01-01 00:00:00 UTC = 1704067200
        assert_eq!(unix_to_ymd(1704067200), "2024-01-01");
        // 2023-12-31 00:00:00 UTC = 1703980800
        assert_eq!(unix_to_ymd(1703980800), "2023-12-31");
        // 2024-02-29 00:00:00 UTC = 1709164800 (leap day)
        assert_eq!(unix_to_ymd(1709164800), "2024-02-29");
    }

    #[test]
    fn extract_fixes_from_body() {
        // Re-test merge.rs's extract_fixes logic here
        let re = regex::Regex::new(r"(?:Fixes|Closes|Resolves)\s+#(\d+)").unwrap();
        let body = "Fixes #42 and Closes #99";
        let fixes: Vec<String> = re
            .captures_iter(body)
            .map(|c| c.get(1).unwrap().as_str().to_string())
            .collect();
        assert_eq!(fixes, vec!["42", "99"]);
    }
}
