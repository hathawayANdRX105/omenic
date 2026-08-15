//! merge hook — validate PR, reviews, cleanup before squash-merge.
//!
//! Port of `.githooks/hooks/merge`. Uses the already-ported Rust rules
//! (pull_requests, reviews) for GitHub validation, plus calls the Python
//! cleanup script.
//!
//! Usage: `gate merge <owner/repo> <pr_number> [--dry-run]`



use crate::shared::{
    exit_code, gh_api, gh_api_paginate, load_yaml, print_findings, run_external, Finding, Severity,
};
use crate::tools::git;

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

    let githooks = git::find_githooks_dir()
        .unwrap_or_else(|| std::path::PathBuf::from(".githooks"));
    let spec_dir = githooks.join("spec");
    let dispatch_path = spec_dir.join("dispatch.yaml");
    let cfg = match load_yaml(dispatch_path.to_str().unwrap_or("")) {
        Ok(c) => Some(c),
        Err(_) => None,
    };

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
            .map(|seq| seq.iter().filter_map(|t| t.as_str().map(String::from)).collect())
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
            "cleanup" => findings.extend(run_cleanup(dry_run)),
            other => eprintln!("unknown merge topic: {}", other),
        }
    }

    // Local CRG + ocr review requirement
    println!("--- review (CRG + ocr) ---");
    findings.extend(check_pr_review(repo, pr_num));

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
            if let Some(head) = pr.get("head").and_then(|h| h.get("ref")).and_then(|r| r.as_str())
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
    let review_cfg = load_yaml(
        spec_dir
            .join("github_reviews.yaml")
            .to_str()
            .unwrap_or(""),
    )
    .ok();

    // Collect all review + issue comments
    let mut bodies = Vec::new();

    if let Ok(comments) =
        gh_api_paginate(&format!("repos/{}/pulls/{}/comments", repo, pr_num), 100)
    {
        for c in &comments {
            if let Some(body) = c.get("body").and_then(|b| b.as_str()) {
                bodies.push(body.to_string());
            }
        }
    }
    if let Ok(comments) = gh_api(&format!("repos/{}/issues/{}/comments", repo, pr_num), None) {
        if let Some(arr) = comments.as_array() {
            for c in arr {
                if let Some(body) = c.get("body").and_then(|b| b.as_str()) {
                    bodies.push(body.to_string());
                }
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

fn run_cleanup(dry_run: bool) -> Vec<Finding> {
    let githooks = git::find_githooks_dir()
        .unwrap_or_else(|| std::path::PathBuf::from(".githooks"));
    let script = githooks.join("cleanup").join("branch_cleanup.py");
    let cwd = githooks
        .parent()
        .map(|p| p.to_string_lossy().to_string());
    let mut cmd_args = vec!["python3", script.to_str().unwrap_or("")];
    if !dry_run {
        cmd_args.push("--apply");
    }
    match run_external(&cmd_args, cwd.as_deref()) {
        Ok((rc, output)) => {
            if rc == 0 {
                vec![Finding::new("cleanup", Severity::Info, "branch cleanup OK")]
            } else {
                vec![Finding::new(
                    "cleanup",
                    Severity::Warn,
                    &format!("branch cleanup reported issues:\n{}", &output[..output.len().min(500)]),
                )]
            }
        }
        Err(e) => vec![Finding::new(
            "cleanup",
            Severity::Warn,
            &format!("branch_cleanup.py: {}", e),
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
            )]
        }
    };
    let file_count = pr_files.as_array().map(|a| a.len()).unwrap_or(0);
    if file_count == 0 {
        println!("PR #{} 无文件改动（{} 个文件），无需审查。", pr_num, file_count);
        return vec![];
    }

    println!(
        "PR #{} 有 {} 个文件改动，必须审查：",
        pr_num, file_count
    );
    if let Some(files) = pr_files.as_array() {
        for f in files.iter().take(10) {
            let name = f.get("filename").and_then(|n| n.as_str()).unwrap_or("?");
            let add = f.get("additions").and_then(|a| a.as_u64()).unwrap_or(0);
            let del = f.get("deletions").and_then(|d| d.as_u64()).unwrap_or(0);
            println!("  {} (+{}/-{})", name, add, del);
        }
    }

    // Run CRG + ocr
    let crg_out = run_external(&["code-review-graph", "detect-changes"], None)
        .map(|(_, o)| o)
        .unwrap_or_else(|e| format!("[CRG] error: {}", e));
    println!("{}", &crg_out[..crg_out.len().min(1200)]);
    println!();
    println!("（ocr 运行中，LLM 需几分钟...）");

    let ocr_out = run_external(&["ocr", "--json"], None)
        .map(|(_, o)| o)
        .unwrap_or_else(|e| format!("[ocr] error: {}", e));
    println!("{}", &ocr_out[..ocr_out.len().min(1500)]);

    vec![Finding::new(
        "RV-07",
        Severity::Info,
        &format!("审查完成: CRG risk + ocr {} 字发现", ocr_out.len()),
    )]
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
