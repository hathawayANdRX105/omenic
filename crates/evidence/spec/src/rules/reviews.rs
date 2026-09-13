//! PR review comment validation.
//!
//! Implements `run`: validates PR review comments against
//! `.githooks/spec/github_reviews.yaml`.
//! All RV-* rules: RV-01 (checkbox forbidden), RV-02 (allowed reply words),
//! RV-03 (reply detail), RV-04 (CRG/inline review format), RV-05 (CRG Review
//! exists), RV-06 (inline findings have reply).

use regex::Regex;
use serde_yaml::Value as YamlValue;

use crate::shared::{Finding, Severity};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn has_cjk(s: &str) -> bool {
    s.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

/// Extract `Agent 🤖 - Fix: <reason>` / `Block:` / ... reply entries.
/// Returns `(intent_word, reason)` pairs.
fn extract_replies(body: &str) -> Vec<(&str, String)> {
    let re = Regex::new(r"Agent 🤖 - (Fix|Block|Resolve|Note|Withdraw|Supersede):\s*(.+)").unwrap();
    re.captures_iter(body)
        .map(|m| {
            let typ = m.get(1).unwrap().as_str();
            let reason = m.get(2).unwrap().as_str().trim().to_string();
            (typ, reason)
        })
        .collect()
}

/// `level` defaults to "unspecified" when the P-level is missing.
fn extract_inline_reviews(body: &str) -> Vec<(&str, String)> {
    let re = Regex::new(r"(?m)Agent 🤖 - Inline Review\s+(P[0-3])?:\s*(.+)").unwrap();
    re.captures_iter(body)
        .map(|m| {
            let level = m.get(1).map(|g| g.as_str()).unwrap_or("unspecified");
            let content = m.get(2).unwrap().as_str().trim().to_string();
            (level, content)
        })
        .collect()
}

/// Extract `## Agent 🤖 - CRG Review: <title>` entries (case-insensitive heading).
fn extract_crg_reviews(body: &str) -> Vec<String> {
    let re = Regex::new(r"(?mi)^## Agent 🤖 - CRG Review:\s*(.+)").unwrap();
    re.captures_iter(body)
        .map(|m| m.get(1).unwrap().as_str().trim().to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// rule function
// ---------------------------------------------------------------------------

/// Validate review comment bodies against `github_reviews.yaml`.
///
/// Port of Python `reviews.run(comments, cfg)`. `comment_bodies` is the list
/// of non-empty comment body strings (Python normalizes `comments` dicts to
/// `bodies` first — callers pass the pre-extracted bodies).
pub fn run(comment_bodies: &[String], cfg: &YamlValue) -> Vec<Finding> {
    let mut findings = Vec::new();
    let bodies: Vec<&str> = comment_bodies.iter().map(|s| s.as_str()).collect();

    // ---- P-22 checkbox forbidden in reviews (RV-01) ----
    let checkbox_re = Regex::new(r"-\s*\[[ xX]\]").unwrap();
    let mut found_checkbox = false;
    for body in &bodies {
        if checkbox_re.is_match(body) {
            findings.push(Finding::new(
                "RV-01",
                Severity::Fail,
                "review comment contains checkbox (- [x] / - [ ])",
            ));
            found_checkbox = true;
            break;
        }
    }
    if !found_checkbox {
        findings.push(Finding::new(
            "RV-01",
            Severity::Info,
            "no checkboxes in review comments",
        ));
    }

    // ---- P-35 review prefix format (RV-04) ----
    let inline_cfg = cfg
        .get("review_formats")
        .and_then(|rf| rf.get("inline_review"))
        .unwrap_or(&YamlValue::Null);
    let allowed_levels: Vec<String> = inline_cfg
        .get("allowed_inline_levels")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_else(|| {
            ["P0", "P1", "P2", "P3"]
                .iter()
                .map(|s| (*s).to_string())
                .collect()
        });

    for body in &bodies {
        for crg in extract_crg_reviews(body) {
            if has_cjk(&crg) {
                findings.push(Finding::new(
                    "RV-04",
                    Severity::Fail,
                    &format!("CRG Review title contains CJK: {crg}"),
                ));
            } else {
                findings.push(Finding::new(
                    "RV-04",
                    Severity::Info,
                    &format!("CRG Review title is English: {crg}"),
                ));
            }
        }

        for ir in extract_inline_reviews(body) {
            let level = ir.0;
            if level != "unspecified" && !allowed_levels.iter().any(|l| l == level) {
                findings.push(Finding::new(
                    "RV-04",
                    Severity::Fail,
                    &format!("Inline Review level '{level}' not in allowed {allowed_levels:?}"),
                ));
            } else {
                findings.push(Finding::new(
                    "RV-04",
                    Severity::Info,
                    &format!("Inline Review prefix OK: level={level}"),
                ));
            }
        }
    }

    // ---- P-24 / P-25 reply threads (RV-02, RV-03) ----
    let mut all_replies: Vec<(&str, String)> = Vec::new();
    for body in &bodies {
        all_replies.extend(extract_replies(body));
    }

    if all_replies.is_empty() {
        findings.push(Finding::new("RV-02", Severity::Info, "no replies to check"));
        findings.push(Finding::new("RV-03", Severity::Info, "no replies to check"));
    } else {
        let crg_cfg = cfg
            .get("review_formats")
            .and_then(|rf| rf.get("crg_review"))
            .unwrap_or(&YamlValue::Null);
        let allowed_reply_words: Vec<String> = crg_cfg
            .get("allowed_reply_words")
            .and_then(|v| v.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_else(|| {
                ["Fix", "Block", "Resolve", "Note", "Withdraw", "Supersede"]
                    .iter()
                    .map(|s| (*s).to_string())
                    .collect()
            });

        let bad: Vec<&str> = all_replies
            .iter()
            .filter(|r| !allowed_reply_words.iter().any(|w| w == r.0))
            .map(|r| r.0)
            .collect();
        if !bad.is_empty() {
            findings.push(Finding::new(
                "RV-02",
                Severity::Warn,
                &format!("some replies use disallowed words: {bad:?}"),
            ));
        } else {
            findings.push(Finding::new(
                "RV-02",
                Severity::Info,
                &format!("all {} reply(ies) use allowed words", all_replies.len()),
            ));
        }

        let short: usize = all_replies.iter().filter(|r| r.1.len() < 5).count();
        if short > 0 {
            findings.push(Finding::new(
                "RV-03",
                Severity::Warn,
                &format!(
                    "{short}/{} replies lack sufficient detail",
                    all_replies.len()
                ),
            ));
        } else {
            findings.push(Finding::new(
                "RV-03",
                Severity::Info,
                &format!("all {} replies have sufficient detail", all_replies.len()),
            ));
        }
    }

    // ---- P-36 CRG Review exists (RV-05) ----
    let has_crg = bodies.iter().any(|b| !extract_crg_reviews(b).is_empty());
    if has_crg {
        findings.push(Finding::new(
            "RV-05",
            Severity::Info,
            "CRG Review present in PR conversation",
        ));
    } else {
        findings.push(Finding::new(
            "RV-05",
            Severity::Fail,
            "no CRG Review comment in PR conversation",
        ));
    }

    // ---- P-37 inline findings have reply (RV-06) ----
    let inline_count: usize = bodies.iter().map(|b| extract_inline_reviews(b).len()).sum();
    if inline_count == 0 {
        findings.push(Finding::new(
            "RV-06",
            Severity::Info,
            "no inline findings to resolve",
        ));
    } else if all_replies.len() >= inline_count {
        findings.push(Finding::new(
            "RV-06",
            Severity::Info,
            &format!(
                "all {} inline finding(s) have reply ({} replies)",
                inline_count,
                all_replies.len()
            ),
        ));
    } else {
        findings.push(Finding::new(
            "RV-06",
            Severity::Fail,
            &format!(
                "{}/{} inline findings have reply — every inline finding MUST have an Agent 🤖 - Fix:/Block:/Resolve:/Note:/Withdraw:/Supersede: reply",
                all_replies.len(),
                inline_count
            ),
        ));
    }

    findings
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------
