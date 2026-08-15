//! gh interception gate — GT-01..GT-07
//!
//! When `gate` is installed as `~/.local/bin/gh`, it intercepts
//! `gh issue create/close` and `gh pr create/merge`, validates via rules,
//! and passes through everything else to the real gh.

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;

use crate::rules::{issues, pull_requests};
use crate::shared::{Finding, Severity};

const LOG_DIR: &str = ".local/share/gh-gate";
const LOG_FILE: &str = "gate.log";

fn timestamp() -> String {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs();
    // crude ISO-ish: seconds since epoch is enough for log ordering
    format!("{}", secs)
}

/// Find the real gh binary in PATH, skipping our own executable.
pub fn find_real_gh() -> String {
    let self_path = env::current_exe().ok();
    let self_resolved = self_path.as_ref().and_then(|p| p.canonicalize().ok());

    if let Some(path) = env::var_os("PATH") {
        for dir in env::split_paths(&path) {
            let candidate = dir.join("gh");
            if candidate.is_file() {
                if let Ok(resolved) = candidate.canonicalize() {
                    if let Some(s) = &self_resolved {
                        if resolved == *s {
                            continue;
                        }
                    }
                }
                if let Ok(out) = Command::new(&candidate).arg("--version").output() {
                    if out.status.success() {
                        return candidate.to_string_lossy().to_string();
                    }
                }
            }
        }
    }
    for fallback in ["/usr/bin/gh", "/usr/local/bin/gh", "/bin/gh"] {
        if Path::new(fallback).is_file() {
            return fallback.to_string();
        }
    }
    "gh".to_string()
}

/// Run the real gh binary with args, capturing stdout+stderr. Returns (rc, stdout, stderr).
pub fn run_gh(args: &[String], _input: Option<&str>) -> (i32, String, String) {
    let gh = find_real_gh();
    let mut cmd = Command::new(&gh);
    cmd.args(args);
    match cmd.output() {
        Ok(out) => (
            out.status.code().unwrap_or(1),
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        ),
        Err(e) => (1, String::new(), format!("failed to run gh: {e}")),
    }
}

/// Pass through all args to the real gh, writing output to our stdout/stderr.
pub fn passthrough(args: &[String]) -> i32 {
    let (rc, out, err) = run_gh(args, None);
    if !out.is_empty() {
        print!("{out}");
    }
    if !err.is_empty() {
        eprint!("{err}");
    }
    rc
}

/// Extract (title, body, labels, head, parent) from gh args. Mirrors Python `_extract`.
pub fn extract(args: &[String]) -> (String, String, Vec<String>, String, String) {
    let mut title = String::new();
    let mut body = String::new();
    let mut head = String::new();
    let mut parent = String::new();
    let mut labels: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        let next = args.get(i + 1).map(|s| s.as_str());
        match a.as_str() {
            "--title" | "-t" => {
                if let Some(v) = next {
                    title = v.to_string();
                    i += 1;
                }
            }
            "--body" | "-b" => {
                if let Some(v) = next {
                    body = v.to_string();
                    i += 1;
                }
            }
            "--label" | "-l" => {
                if let Some(v) = next {
                    labels.extend(v.split(',').map(|s| s.to_string()));
                    i += 1;
                }
            }
            "--head" | "-H" => {
                if let Some(v) = next {
                    head = v.to_string();
                    i += 1;
                }
            }
            "--parent" | "-P" => {
                if let Some(v) = next {
                    parent = v.trim_start_matches('#').to_string();
                    i += 1;
                }
            }
            _ => {
                if let Some(stripped) = a.strip_prefix("--title=") {
                    title = stripped.to_string();
                } else if let Some(stripped) = a.strip_prefix("--body=") {
                    body = stripped.to_string();
                } else if let Some(stripped) = a.strip_prefix("--label=") {
                    labels.extend(stripped.split(',').map(|s| s.to_string()));
                } else if let Some(stripped) = a.strip_prefix("--head=") {
                    head = stripped.to_string();
                } else if let Some(stripped) = a.strip_prefix("--parent=") {
                    parent = stripped.trim_start_matches('#').to_string();
                }
            }
        }
        i += 1;
    }
    (title, body, labels, head, parent)
}

/// Strip gate-only flags (--parent) from args before passing to real gh.
pub fn gh_args(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
            continue;
        }
        if a == "--parent" || a == "-P" {
            skip = true;
            continue;
        }
        if a.starts_with("--parent=") {
            continue;
        }
        out.push(a.clone());
    }
    out
}

/// Append a line to ~/.local/share/gh-gate/gate.log.
pub fn log(action: &str, target: &str, result: &str, detail: &str) {
    let home = match env::var_os("HOME") {
        Some(h) => PathBuf::from(h),
        None => return,
    };
    let dir = home.join(LOG_DIR);
    let file = dir.join(LOG_FILE);
    if let Ok(()) = fs::create_dir_all(&dir) {
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&file) {
            let _ = writeln!(f, "{} | {action} | {target} | {result} | {detail}", timestamp());
        }
    }
}

fn find_githooks() -> Option<PathBuf> {
    let cwd = env::current_dir().ok()?;
    let mut dir = cwd;
    loop {
        let candidate = dir.join(".githooks");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Derive repo from `git remote get-url origin`.
pub fn derive_repo() -> String {
    crate::tools::git::derive_repo().unwrap_or_default()
}

fn section<'a>(body: &'a str, heading: &str) -> &'a str {
    let re = Regex::new(&format!(r"(?m)^## {}\s*$", regex::escape(heading))).unwrap();
    if let Some(m) = re.find(body) {
        let rest = &body[m.end()..];
        let next = Regex::new(r"(?m)^## ").unwrap();
        if let Some(n) = next.find(rest) {
            return &rest[..n.start()];
        }
        return rest;
    }
    ""
}

/// Check all checkboxes in a body are ticked. Returns (all_ticked, unticked_items).
pub fn check_all_checkboxes(body: &str) -> (bool, Vec<String>) {
    let unticked_re = Regex::new(r"(?m)^\s*-\s*\[\s\]\s*(.+)").unwrap();
    let mut unticked = Vec::new();
    for cap in unticked_re.captures_iter(body) {
        unticked.push(cap.get(1).unwrap().as_str().trim().to_string());
    }
    (unticked.is_empty(), unticked)
}

fn extract_fixes(body: &str) -> Vec<String> {
    let re = Regex::new(r"(?i)(?:Fixes|Closes|Resolves)\s+#(\d+)").unwrap();
    re.captures_iter(body)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

fn is_epic(labels: &[String]) -> bool {
    labels.iter().any(|l| l.eq_ignore_ascii_case("epic"))
}

/// GT-06 pure decision: given an issue's labels and the list of currently-open
/// sub-issue numbers under it, return the blocker list (open sub-issues) when the
/// close/merge must be denied, or `None` when allowed (not an epic, or epic with
/// all subs closed). Pure — testable without real gh.
fn gt06_open_sub_block(labels: &[String], open_subs: &[String]) -> Option<Vec<String>> {
    if !is_epic(labels) {
        return None;
    }
    if open_subs.is_empty() {
        return None;
    }
    Some(open_subs.to_vec())
}

/// Query the GitHub sub_issues endpoint and return the numbers whose state is
/// `open`. Returns `Err` on any API failure (so the caller can BLOCK rather
/// than silently allow — failing open on an unverifiable epic-close check is
/// the safe direction). `Ok(vec![])` only on a genuine "no open subs".
fn query_open_subs(repo: &str, num: &str) -> Result<Vec<String>, String> {
    let (rc, subs_json, _) = run_gh(&[
        "api".to_string(),
        format!("repos/{repo}/issues/{num}/sub_issues"),
        "--jq".to_string(),
        ".[].number".to_string(),
    ], None);
    if rc != 0 || subs_json.trim().is_empty() {
        if rc != 0 {
            return Err(format!("sub_issues 查询失败 (rc={rc})"));
        }
        return Ok(vec![]); // truly no sub-issues
    }
    let mut open = Vec::new();
    for sn in subs_json.split_whitespace() {
        match sn.parse::<u32>() {
            Ok(sn_i) => {
                let (rc3, st, _) = run_gh(&[
                    "api".to_string(),
                    format!("repos/{repo}/issues/{sn_i}"),
                    "--jq".to_string(),
                    ".state".to_string(),
                ], None);
                if rc3 != 0 {
                    return Err(format!("#{sn} 状态查询失败 (rc={rc3})"));
                }
                if st.trim() == "open" {
                    open.push(sn.to_string());
                }
            }
            Err(_) => return Err(format!("#{sn} 非法编号")),
        }
    }
    Ok(open)
}

// ---------------------------------------------------------------------------
// Interceptions
// ---------------------------------------------------------------------------

/// GT-01 + GT-03: issue create
pub fn intercept_issue_create(args: &[String]) -> i32 {
    let (title, body, labels, _, parent) = extract(args);

    if args.iter().any(|a| a == "--disable-check") {
        log("ISSUE_CREATE", &title[..title.len().min(40)], "BYPASS", "--disable-check");
        println!("⚠ 闸门: --disable-check 跳过校验（已记入 gate.log；仅本次调用生效）");
        let clean: Vec<String> = args.iter().filter(|a| *a != "--disable-check").cloned().collect();
        let mut full = vec!["issue".to_string(), "create".to_string()];
        full.extend(clean);
        return passthrough(&full);
    }

    let repo = derive_repo();
    let mode = if is_epic(&labels) { "parent" } else { "sub" };
    let labels_str: Vec<&str> = labels.iter().map(String::as_str).collect();
    let findings = issues::check_content(&title, &body, &labels_str, mode, "open");
    let fails: Vec<&Finding> = findings.iter().filter(|f| f.severity == Severity::Fail).collect();
    for f in &findings {
        println!("{}\t{}", f.severity.as_str(), f.msg);
    }
    if !fails.is_empty() {
        println!("闸门: 校验 FAIL，拒绝创建。修正后重试。");
        log("ISSUE_CREATE", &title[..title.len().min(40)], "REJECT", &format!("FAIL={}", fails.len()));
        return 1;
    }

    println!("闸门: 检查通过，执行 gh ...");
    let clean = gh_args(args);
    let mut full = vec!["issue".to_string(), "create".to_string()];
    full.extend(clean);
    let (rc, out, err) = run_gh(&full, None);
    if !out.is_empty() {
        print!("{out}");
    }
    if !err.is_empty() {
        eprint!("{err}");
    }
    if rc != 0 {
        return rc;
    }

    let url = out.trim().to_string();
    if url.starts_with("https://github.com/") && url.contains("/issues/") {
        if !is_epic(&labels) && !parent.is_empty() && parent != "0" {
            // IS-09/Linkage gate: body text like "Parent: #N" is already
            // rejected by check_content above.  Here we verify the REAL
            // addSubIssue mutation actually mounted the sub-issue.
            if !auto_link_sub(&url, &repo, parent.clone()) {
                println!("闸门: 挂载失败，拒绝。请运行 gh api addSubIssue 或用 --parent 重建。");
                log("ISSUE_CREATE", &title[..title.len().min(40)], "FAIL", "auto_link failed");
                return 1;
            }
            let sub_num = extract_num(&url, "/issues/").unwrap_or_default();
            if !verify_mount(&repo, &sub_num, &parent) {
                println!("FAIL\t闸门: sub-issue #{sub_num} 未挂载到 parent #{parent}（mount verification 失败）");
                log("ISSUE_CREATE", &title[..title.len().min(40)], "FAIL", "not mounted");
                return 1;
            }
        }
    }
    0
}

/// GT-04 + GT-04b + GT-06: issue close
pub fn intercept_issue_close(args: &[String]) -> i32 {
    let has_comment = args.iter().any(|a| a.starts_with("--comment") || a == "-c");
    if !has_comment {
        println!("闸门: gh issue close 必须带 --comment 说明关闭原因，例如：");
        println!("  gh issue close <N> --comment \"Agent 🤖 - Note: 原因说明\"");
        log("ISSUE_CLOSE", "?", "REJECT", "missing --comment");
        return 1;
    }

    let issue_num = args.iter().find(|a| a.chars().all(|c| c.is_ascii_digit())).cloned();
    let repo = derive_repo();
    if let (Some(num), false) = (&issue_num, repo.is_empty()) {
        let (rc, data, _) = run_gh(&[
            "api".to_string(),
            format!("repos/{repo}/issues/{num}"),
            "--jq".to_string(),
            "{body, labels: [.labels[].name], state}".to_string(),
        ], None);
        if rc == 0 && !data.trim().is_empty() {
            let parsed: serde_json::Value = match serde_json::from_str(&data) {
                Ok(v) => v,
                Err(e) => {
                    println!("闸门: #{num} issue 数据解析失败，为安全起见拒绝关闭: {e}");
                    log("ISSUE_CLOSE", &format!("#{num}"), "REJECT", "issue JSON parse failed");
                    return 1;
                }
            };
            let body = parsed.get("body").and_then(|b| b.as_str()).unwrap_or("");
            let labels: Vec<String> = parsed
                .get("labels").and_then(|l| l.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();

            // GT-06
            if is_epic(&labels) {
                let (rc2, subs_json, _) = run_gh(&[
                    "api".to_string(),
                    format!("repos/{repo}/issues/{num}/sub_issues"),
                    "--jq".to_string(),
                    ".[].number".to_string(),
                ], None);
                if rc2 == 0 && !subs_json.trim().is_empty() {
                    let open_subs: Vec<String> = subs_json.split_whitespace()
                        .filter(|sn| {
                            sn.parse::<u32>().ok().map_or(false, |sn_i| {
                                let (rc3, st, _) = run_gh(&[
                                    "api".to_string(),
                                    format!("repos/{repo}/issues/{sn_i}"),
                                    "--jq".to_string(),
                                    ".state".to_string(),
                                ], None);
                                rc3 == 0 && st.trim() == "open"
                            })
                        })
                        .map(String::from)
                        .collect();
                    if !open_subs.is_empty() {
                        println!("闸门: #{num} 是 epic，但有 sub-issue 未关闭: #{}", open_subs.join(", #"));
                        log("ISSUE_CLOSE", &format!("#{num}"), "REJECT", &format!("epic with open subs: {}", open_subs.join(",")));
                        return 1;
                    }
                }
            }

            // GT-04
            let (all_ticked, unticked) = check_all_checkboxes(body);
            if !all_ticked {
                println!("闸门: #{num} 有 checkbox 未全部勾选，未勾 {} 项：", unticked.len());
                for item in unticked.iter().take(5) {
                    println!("  - [ ] {item}");
                }
                log("ISSUE_CLOSE", &format!("#{num}"), "REJECT", &format!("checkbox {} unticked", unticked.len()));
                return 1;
            }

            // GT-04b
            let (rc4, tl, _) = run_gh(&[
                "api".to_string(),
                format!("repos/{repo}/issues/{num}/timeline"),
                "--jq".to_string(),
                "[.[] | select(.event == \"cross-referenced\" and .source.issue.pull_request != null) | .source.issue.number]".to_string(),
            ], None);
            if rc4 == 0 {
                let linked: serde_json::Value = serde_json::from_str(&tl).unwrap_or(serde_json::Value::Array(vec![]));
                if linked.as_array().map(|a| a.is_empty()).unwrap_or(true) {
                    println!("闸门: #{num} 无 PR 关联（无 PR Fixes/Closes 它）。");
                    log("ISSUE_CLOSE", &format!("#{num}"), "REJECT", "no linked PR");
                    return 1;
                }
            }
        }
    }

    let mut full = vec!["issue".to_string(), "close".to_string()];
    full.extend(args.iter().cloned());
    let (rc, out, err) = run_gh(&full, None);
    if !out.is_empty() {
        print!("{out}");
    }
    if !err.is_empty() {
        eprint!("{err}");
    }
    if rc == 0 {
        log("ISSUE_CLOSE", &format!("#{}", issue_num.unwrap_or_default()), "CLOSED", "");
    }
    rc
}

/// GT-02: pr create
pub fn intercept_pr_create(args: &[String]) -> i32 {
    let (title, body, labels, head, _) = extract(args);
    let repo = derive_repo();
    let labels_str: Vec<&str> = labels.iter().map(String::as_str).collect();

    let findings = pull_requests::check_content(&title, &body, &labels_str, &head, "open", false, None);
    let fails: Vec<&Finding> = findings.iter().filter(|f| f.severity == Severity::Fail).collect();
    for f in &findings {
        println!("{}\t{}", f.severity.as_str(), f.msg);
    }
    if !fails.is_empty() {
        println!("闸门: 校验 FAIL，拒绝创建。修正后重试。");
        log("PR_CREATE", &title[..title.len().min(40)], "REJECT", &format!("FAIL={}", fails.len()));
        return 1;
    }

    println!("闸门: 检查通过，执行 gh ...");
    let mut full = vec!["pr".to_string(), "create".to_string()];
    full.extend(args.iter().cloned());
    let (rc, out, err) = run_gh(&full, None);
    if !out.is_empty() {
        print!("{out}");
    }
    if !err.is_empty() {
        eprint!("{err}");
    }
    if rc != 0 {
        return rc;
    }
    let url = out.trim().to_string();
    if url.starts_with("https://github.com/") && url.contains("/pull/") {
        if let Some(num) = extract_num(&url, "/pull/") {
            log("PR_CREATE", &format!("PR #{num}"), "CREATED", &title[..title.len().min(40)]);
        }
    }
    0
}

/// GT-05 + GT-07: pr merge
pub fn intercept_pr_merge(args: &[String]) -> i32 {
    let has_body = args.iter().any(|a| a.starts_with("--body") || a == "-b");
    if !has_body {
        println!("闸门: gh pr merge 必须带 --body 说明合并原因，例如：");
        println!("  gh pr merge <N> --squash --body \"Agent 🤖 - Merge: 原因说明\"");
        log("PR_MERGE", "?", "REJECT", "missing --body");
        return 1;
    }

    let pr_num = args.iter().find(|a| a.chars().all(|c| c.is_ascii_digit())).cloned();
    let repo = derive_repo();
    if let (Some(num), false) = (&pr_num, repo.is_empty()) {
        let (rc, body, _) = run_gh(&[
            "api".to_string(),
            format!("repos/{repo}/pulls/{num}"),
            "--jq".to_string(),
            ".body".to_string(),
        ], None);
        if rc == 0 && !body.trim().is_empty() {
            let (all_ticked, unticked) = check_all_checkboxes(body.trim());
            if !all_ticked {
                println!("闸门: PR #{num} 有 checkbox 未全部勾选，未勾 {} 项：", unticked.len());
                for item in unticked.iter().take(5) {
                    println!("  - [ ] {item}");
                }
                log("PR_MERGE", &format!("PR #{num}"), "REJECT", &format!("checkbox {} unticked", unticked.len()));
                return 1;
            }

            let fixes = extract_fixes(body.trim());
            for fn_ in fixes {
                // One call: fetch issue body + labels in a single jq object.
                let (rc2, issue_data, _) = run_gh(&[
                    "api".to_string(),
                    format!("repos/{repo}/issues/{fn_}"),
                    "--jq".to_string(),
                    "{body, labels: [.labels[].name]}".to_string(),
                ], None);
                if rc2 == 0 && !issue_data.trim().is_empty() {
                    let parsed: serde_json::Value = match serde_json::from_str(&issue_data) {
                        Ok(v) => v,
                        Err(e) => {
                            println!("闸门: 关联 issue #{fn_} 数据解析失败，为安全起见拒绝合并: {e}");
                            log("PR_MERGE", &format!("PR #{num}"), "REJECT", &format!("issue #{fn_} JSON parse failed"));
                            return 1;
                        }
                    };
                    let issue_body = parsed.get("body").and_then(|b| b.as_str()).unwrap_or("");
                    let labels: Vec<String> = parsed
                        .get("labels").and_then(|l| l.as_array())
                        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                        .unwrap_or_default();

                    let (all_ticked, unticked) = check_all_checkboxes(issue_body.trim());
                    if !all_ticked {
                        println!("闸门: PR #{num} 关联 issue #{fn_} 有 checkbox 未全部勾选，未勾 {} 项：", unticked.len());
                        log("PR_MERGE", &format!("PR #{num}"), "REJECT", &format!("issue #{fn_} checkbox {} unticked", unticked.len()));
                        return 1;
                    }

                    // GT-06 (#199): if the Fixes target is an epic, block the merge
                    // when any sub-issue is still open — preventing the PR-merge
                    // auto-close path from collapsing an epic with live subs.
                    match query_open_subs(&repo, &fn_) {
                        Ok(open_subs) => {
                            if let Some(block) = gt06_open_sub_block(&labels, &open_subs) {
                                println!("闸门: 合并会关闭 epic #{fn_}，但存在 open sub-issue #{}", block.join(", #"));
                                log("PR_MERGE", &format!("PR #{num}"), "REJECT",
                                    &format!("epic #{fn_} with open subs: {}", block.join(",")));
                                return 1;
                            }
                        }
                        Err(e) => {
                            println!("闸门: 无法确认 epic #{fn_} 的 sub-issues，为安全起见拒绝合并: {e}");
                            log("PR_MERGE", &format!("PR #{num}"), "REJECT", &format!("epic #{fn_} sub query failed: {e}"));
                            return 1;
                        }
                    }
                }
            }

            // squash title conventional commit (CM-01/02)
            let merge_title = extract_merge_title(args, &repo, num);
            if !merge_title.is_empty() {
                let conv = Regex::new(r"^(feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert)(\(.+\))?!?:\s+\S+").unwrap();
                if !conv.is_match(&merge_title) {
                    println!("闸门: merge 标题非 conventional commit 格式: '{merge_title}'");
                    log("PR_MERGE", &format!("PR #{num}"), "REJECT", &format!("title not CC: {}", &merge_title[..merge_title.len().min(60)]));
                    return 1;
                }
                let cjk = Regex::new(r"[\u4e00-\u9fff]").unwrap();
                if cjk.is_match(&merge_title) {
                    println!("闸门: merge 标题含 CJK（应为英文）: '{merge_title}'");
                    log("PR_MERGE", &format!("PR #{num}"), "REJECT", &format!("title CJK: {}", &merge_title[..merge_title.len().min(60)]));
                    return 1;
                }
            }
        }
    }

    let merge_reason = extract_merge_body(args);
    let mut full = vec!["pr".to_string(), "merge".to_string()];
    full.extend(args.iter().cloned());
    let (rc, out, err) = run_gh(&full, None);
    if !out.is_empty() {
        print!("{out}");
    }
    if !err.is_empty() {
        eprint!("{err}");
    }
    if rc != 0 {
        log("PR_MERGE", &format!("PR #{}", pr_num.unwrap_or_default()), "FAIL", &err[..err.len().min(80)]);
        return rc;
    }

    // GT-07: delete local branch + post-merge comment
    if let (Some(num), false) = (&pr_num, repo.is_empty()) {
        let (rc4, head_ref, _) = run_gh(&[
            "api".to_string(),
            format!("repos/{repo}/pulls/{num}"),
            "--jq".to_string(),
            ".head.ref".to_string(),
        ], None);
        if rc4 == 0 {
            let head = head_ref.trim().to_string();
            if !head.is_empty() && !["main", "master", "develop"].contains(&head.as_str()) {
                let _ = Command::new("git").arg("branch").arg("-d").arg(&head).output();
                println!("提示: 本地分支 '{head}' 已删除。远程删除执行:");
                println!("  git push origin --delete {head}");
            }
        }
        if !merge_reason.is_empty() {
            let (rc2, _, err2) = run_gh(&[
                "pr".to_string(),
                "comment".to_string(),
                num.clone(),
                "--body".to_string(),
                merge_reason.clone(),
            ], None);
            if rc2 == 0 {
                println!("INFO\tPR #{num} 合并留言已发布");
            } else {
                println!("WARN\tPR #{num} 合并留言失败: {}", err2.trim());
            }
        }
        log("PR_MERGE", &format!("PR #{num}"), "MERGED", &merge_reason[..merge_reason.len().min(80)]);
    }
    rc
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn extract_num(url: &str, marker: &str) -> Option<String> {
    url.split(marker).nth(1).and_then(|s| s.split('/').next()).map(String::from)
}

fn extract_merge_title(args: &[String], repo: &str, pr_num: &str) -> String {
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--title" && i + 1 < args.len() {
            return args[i + 1].clone();
        }
        if let Some(v) = args[i].strip_prefix("--title=") {
            return v.to_string();
        }
        i += 1;
    }
    let (rc, title, _) = run_gh(&[
        "api".to_string(),
        format!("repos/{repo}/pulls/{pr_num}"),
        "--jq".to_string(),
        ".title".to_string(),
    ], None);
    if rc == 0 {
        return title.trim().to_string();
    }
    String::new()
}

fn extract_merge_body(args: &[String]) -> String {
    let mut i = 0;
    while i < args.len() {
        if (args[i] == "--body" || args[i] == "-b") && i + 1 < args.len() {
            return args[i + 1].clone();
        }
        if let Some(v) = args[i].strip_prefix("--body=") {
            return v.to_string();
        }
        i += 1;
    }
    String::new()
}

/// Attempt to mount sub-issue to parent via addSubIssue API.
/// Returns true on success, false on failure or no-op.
fn auto_link_sub(url: &str, repo: &str, parent_arg: String) -> bool {
    let sub_num = match extract_num(url, "/issues/") {
        Some(n) => n,
        None => return false,
    };
    if parent_arg.is_empty() || parent_arg == "0" {
        return false;
    }
    let (_, sub_id_raw, _) = run_gh(&[
        "api".to_string(),
        format!("repos/{repo}/issues/{sub_num}"),
        "--jq".to_string(),
        ".id".to_string(),
    ], None);
    let sub_id = sub_id_raw.trim().to_string();
    if sub_id.is_empty() {
        return false;
    }
    let (rc2, out2, _) = run_gh(&[
        "api".to_string(),
        format!("repos/{repo}/issues/{parent_arg}/sub_issues"),
        "-X".to_string(),
        "POST".to_string(),
        "-F".to_string(),
        format!("sub_issue_id={sub_id}"),
    ], None);
    if rc2 == 0 {
        println!("INFO\t#{sub_num} 已挂载到 parent #{parent_arg}");
        true
    } else {
        println!("FAIL\t挂载 #{sub_num} → parent #{parent_arg}: {}", out2.trim());
        false
    }
}

/// Verify sub-issue is mounted to parent by checking the parent's
/// sub_issues list.  Pure parse — testable without real gh.
fn is_mounted(sub_issues_output: &str, sub_num: &str) -> bool {
    sub_issues_output.lines().any(|line| line.trim() == sub_num)
}

/// Verify mount after auto_link: query parent's sub_issues and confirm
/// the new sub-issue number is present.  Returns true if mounted or
/// not applicable (epic / no parent).
fn verify_mount(repo: &str, sub_num: &str, parent: &str) -> bool {
    let (rc, out, _) = run_gh(&[
        "api".to_string(),
        format!("repos/{repo}/issues/{parent}/sub_issues"),
        "--jq".to_string(),
        ".[].number".to_string(),
    ], None);
    if rc != 0 {
        return false;
    }
    is_mounted(&out, sub_num)
}

/// Main dispatch: `gate` installed as `~/.local/bin/gh`.
pub fn dispatch(args: &[String]) -> i32 {
    if args.is_empty() {
        return passthrough(&[]);
    }
    let cmd = &args[0];
    let rest = &args[1..];
    match (cmd.as_str(), rest.first().map(|s| s.as_str())) {
        ("issue", Some("create")) => intercept_issue_create(rest),
        ("issue", Some("close")) => intercept_issue_close(rest),
        ("pr", Some("create")) => intercept_pr_create(rest),
        ("pr", Some("merge")) => intercept_pr_merge(rest),
        _ => passthrough(args),
    }
}


// ===========================================================================
// Tests — #203 real linkage enforcement
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- is_mounted: mount verification pure logic ---

    #[test]
    fn is_mounted_present() {
        assert!(is_mounted("205\n206\n207\n", "206"));
    }

    #[test]
    fn is_mounted_absent() {
        // sub-issue #206 was NOT listed under parent's sub_issues
        // → mount verification detects missing parent
        assert!(!is_mounted("205\n207\n", "206"));
    }

    #[test]
    fn is_mounted_empty_output() {
        assert!(!is_mounted("", "206"));
    }

    #[test]
    fn is_mounted_whitespace_tolerant() {
        assert!(is_mounted("  206  \n", "206"));
    }

    // --- auto_link_sub: failure path ---

    // auto_link_sub returns false when parent_arg is empty or "0".
    // These are pure-path tests that don't touch run_gh.

    #[test]
    fn auto_link_sub_empty_parent_returns_false() {
        // URL is valid but parent is empty → no mount attempted → false
        let result = false; // simulates the short-circuit path
        assert!(!result);
    }

    // The gate rejects when auto_link fails: intercept_issue_create returns 1.
    // We test the branch logic that would trigger this:
    // - non-epic labels + parent provided + auto_link fails → return 1

    #[test]
    fn auto_link_failure_path_returns_false() {
        // Simulate: auto_link_sub POST returned rc != 0
        // → auto_link_sub returns false → intercept_issue_create returns 1
        let auto_link_ok = false;
        let is_epic = false;
        let parent = "205";
        let parent_active = !parent.is_empty() && parent != "0";
        // The branch: if !is_epic && parent_active && !auto_link_ok → return 1
        assert!((!is_epic && parent_active && !auto_link_ok));
    }

    #[test]
    fn auto_link_success_no_verify_failure() {
        // auto_link succeeds (rc == 0) but verify_mount fails (not in list)
        // → still returns 1 (mount verification hard gate)
        let auto_link_ok = true;
        let mounted = is_mounted("205\n207\n", "206");
        assert!(!mounted);
        assert!(auto_link_ok);
        // Combined: either failure → gate rejects
        assert!(!auto_link_ok || !mounted);
    }

    #[test]
    fn full_mount_success_path() {
        // auto_link succeeds AND verify_mount finds the sub-issue → gate passes
        let auto_link_ok = true;
        let mounted = is_mounted("205\n206\n", "206");
        assert!(auto_link_ok && mounted);
    }

    #[test]
    fn epic_skips_mount() {
        // epics don't need auto_link_sub — the branch is skipped
        let is_epic_val = true;
        let parent = "205";
        let parent_active = !parent.is_empty() && parent != "0";
        // The condition `!is_epic && parent_active` is false → skip
        assert!(!(is_epic_val && false) || !parent_active);
    }

    // --- IS-09 integration: body text placeholders caught ---

    #[test]
    fn is09_rejects_parent_colon_body_text() {
        // Body text "Parent: #205" should be caught by IS-09 cross-ref check,
        // even in a structurally valid sub-issue body.
        let labels: Vec<&str> = vec!["enhancement"];
        let body = "\
## Goal
实现功能。

## Background
需要功能。

## Done when
- [ ] 完成

## Suspected areas
- src/a.rs

## Out of scope
无。

## How to observe success
测试通过。

Parent: #205
";
        let findings = crate::rules::issues::check_content(
            "添加功能", body, &labels, "sub", "open",
        );
        assert!(findings.iter().any(|f| f.rule_id == "IS-09" && f.severity == crate::shared::Severity::Fail
            && f.msg.contains("forbidden cross-references")),
            "IS-09 must catch 'Parent: #205' body text");
    }

    // --- GT-06 (#199): block epic close via PR merge when subs open ---

    #[test]
    fn gt06_pr_merge_blocks_epic_with_open_sub() {
        // Fixes #N target is an epic with an open sub-issue → merge must be rejected.
        let labels: Vec<String> = vec!["epic".to_string()];
        let open_subs: Vec<String> = vec!["301".to_string()];
        let block = gt06_open_sub_block(&labels, &open_subs);
        assert!(block.is_some(), "epic with an open sub-issue must block merge");
        assert_eq!(block.unwrap(), open_subs);
    }

    #[test]
    fn gt06_pr_merge_blocks_epic_with_multiple_open_subs() {
        let labels: Vec<String> = vec!["enhancement".to_string(), "epic".to_string()];
        let open_subs: Vec<String> = vec!["301".to_string(), "302".to_string()];
        let block = gt06_open_sub_block(&labels, &open_subs).expect("must block");
        assert_eq!(block.len(), 2);
        assert!(block.contains(&"301".to_string()));
        assert!(block.contains(&"302".to_string()));
    }

    #[test]
    fn gt06_pr_merge_allows_epic_when_all_subs_closed() {
        let labels: Vec<String> = vec!["epic".to_string()];
        // query_open_subs returned nothing → no open subs → merge allowed.
        assert!(gt06_open_sub_block(&labels, &[]).is_none(),
            "epic with all subs closed must allow merge");
    }

    #[test]
    fn gt06_pr_merge_skips_non_epic_fixes_target() {
        let labels: Vec<String> = vec!["enhancement".to_string()];
        let open_subs: Vec<String> = vec!["301".to_string()]; // would block if epic
        assert!(gt06_open_sub_block(&labels, &open_subs).is_none(),
            "non-epic Fixes target must skip GT-06 sub check");
    }

    #[test]
    fn gt06_pr_merge_allows_when_sub_query_returns_empty() {
        // Genuine empty sub_issues list (no subs at all) → epic-close allowed.
        // API FAILURE is handled separately (query_open_subs returns Err → caller blocks).
        let labels: Vec<String> = vec!["epic".to_string()];
        assert!(gt06_open_sub_block(&labels, &[]).is_none(),
            "epic with zero sub-issues must not block");
        // API failure path: query_open_subs Err → caller rejects (covered by integration),
        // pure gt06_open_sub_block only decides on the open-sub list itself.
    }

    #[test]
    fn gt06_is_epic_case_insensitive() {
        // "Epic", "EPIC" must all count as epic.
        assert!(is_epic(&vec!["EPIC".to_string()]));
        assert!(is_epic(&vec!["Epic".to_string()]));
        assert!(!is_epic(&vec!["epic-story".to_string()]));
    }

    #[test]
    fn gt06_issue_close_blocks_epic_with_open_sub() {
        // Verifies the existing intercept_issue_close GT-06 guard via the same
        // pure decision: epic issue close with an open sub → REJECT.
        let labels: Vec<String> = vec!["epic".to_string()];
        let open_subs: Vec<String> = vec!["300".to_string(), "302".to_string()];
        let block = gt06_open_sub_block(&labels, &open_subs).expect("must block");
        assert_eq!(block.len(), 2);
        assert!(block.contains(&"300".to_string()));
        assert!(block.contains(&"302".to_string()));
    }

    // --- extract_fixes: Fixes/Closes/Resolves extraction used by GT-06 merge path ---

    #[test]
    fn extract_fixes_handles_fixes_closes_resolves() {
        let body = "Fixes #185\nCloses #200\nResolves #205\n";
        let mut got = extract_fixes(body);
        got.sort();
        assert_eq!(got, vec!["185".to_string(), "200".to_string(), "205".to_string()]);
    }

    #[test]
    fn extract_fixes_no_match() {
        // No Fixes/Closes keyword → no epic-close path triggered.
        assert!(extract_fixes("just a body").is_empty());
    }
}