//! Branch cleanup: CL-01 stale branch detection and removal.

use std::path::Path;

use crate::gate::shared::{Finding, Severity, load_yaml, run_external};

#[derive(Debug, Clone)]
struct CleanupConfig {
    protected_branches: Vec<String>,
    local_merged_action: String,
    remote_merged_action: String,
    orphan_local_action: String,
    temp_branch_action: String,
    temp_branch_prefixes: Vec<String>,
}

impl CleanupConfig {
    fn from_yaml(value: &serde_yaml::Value) -> Self {
        let protected_branches = strings(value, "protected_branches", &["main", "master", "dev"]);
        let temp_branch_prefixes =
            strings(value, "temp_branch_prefixes", &["tmp/", "wip/", "test/"]);
        Self {
            protected_branches,
            local_merged_action: action(value, "local_merged"),
            remote_merged_action: action(value, "remote_merged"),
            orphan_local_action: action(value, "orphan_local"),
            temp_branch_action: action(value, "temp_branch"),
            temp_branch_prefixes,
        }
    }
}

fn strings(value: &serde_yaml::Value, key: &str, default: &[&str]) -> Vec<String> {
    value
        .get(key)
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .filter(|items: &Vec<String>| !items.is_empty())
        .unwrap_or_else(|| default.iter().map(|s| (*s).to_string()).collect())
}

fn action(value: &serde_yaml::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|v| v.get("action"))
        .and_then(|v| v.as_str())
        .unwrap_or("WARN")
        .to_string()
}

fn run_git(args: &[&str]) -> Result<(i32, String), String> {
    let mut cmd = vec!["git"];
    cmd.extend(args);
    run_external(&cmd, None)
}

fn branch_marker_trimmed(line: &str) -> &str {
    line.trim().trim_start_matches(['*', '+']).trim()
}

fn branch_names(output: &str) -> impl Iterator<Item = String> + '_ {
    output
        .lines()
        .map(branch_marker_trimmed)
        .map(str::to_string)
        .filter(|line| !line.is_empty())
}

fn is_protected(branch: &str, protected: &[String], current: &str) -> bool {
    branch == current || protected.iter().any(|p| p == branch)
}

fn should_skip_merged_branch(
    branch: &str,
    base_branch: &str,
    protected: &[String],
    current: &str,
) -> bool {
    branch == base_branch || is_protected(branch, protected, current)
}

fn delete_branch(
    findings: &mut Vec<Finding>,
    deleted: &mut Vec<String>,
    label: String,
    args: &[&str],
) {
    match run_git(args) {
        Ok((0, _)) => deleted.push(label),
        Ok((rc, output)) => findings.push(Finding::new(
            "CL-01",
            Severity::Warn,
            &format!("failed to delete '{label}' (rc={rc}): {output}"),
        )),
        Err(error) => findings.push(Finding::new(
            "CL-01",
            Severity::Warn,
            &format!("failed to delete '{label}': {error}"),
        )),
    }
}

fn orphan_branch_name<'a>(line: &'a str, protected: &[String], current: &str) -> Option<&'a str> {
    let branch_line = branch_marker_trimmed(line);
    let name = branch_line.split_whitespace().next()?;
    if is_protected(name, protected, current) {
        return None;
    }
    let has_upstream = branch_line.contains('[');
    if has_upstream && !branch_line.contains(": gone]") {
        return None;
    }
    Some(name)
}

fn branch_from_origin_head(output: &str) -> Option<&str> {
    output
        .trim()
        .strip_prefix("origin/")
        .filter(|s| !s.is_empty())
}

fn default_branch() -> String {
    if let Ok((0, output)) = run_git(&[
        "symbolic-ref",
        "--quiet",
        "--short",
        "refs/remotes/origin/HEAD",
    ]) && let Some(branch) = branch_from_origin_head(&output)
    {
        return branch.to_string();
    }
    for branch in ["main", "master"] {
        if let Ok((0, _)) = run_git(&["rev-parse", "--verify", branch]) {
            return branch.to_string();
        }
    }
    "main".to_string()
}

pub fn run(dry_run: bool) -> Vec<Finding> {
    let githooks = crate::gate::tools::git::find_githooks_dir()
        .unwrap_or_else(|| Path::new(".githooks").to_path_buf());
    let spec_path = githooks.join("spec/cleanup_branch_cleanup.yaml");
    let yaml = match load_yaml(spec_path.to_str().unwrap_or("")) {
        Ok(value) => value,
        Err(error) => {
            return vec![Finding::new(
                "CL-01",
                Severity::Warn,
                &format!("failed to load cleanup config: {error}"),
            )];
        }
    };
    let cfg = CleanupConfig::from_yaml(&yaml);
    let current_branch = run_git(&["branch", "--show-current"])
        .ok()
        .and_then(|(rc, out)| (rc == 0).then(|| out.trim().to_string()))
        .unwrap_or_default();
    let base_branch = default_branch();
    let remote_base = format!("origin/{base_branch}");

    let mut findings = Vec::new();
    let mut deleted = Vec::new();

    match run_git(&["branch", "--merged", base_branch.as_str()]) {
        Ok((0, output)) => {
            for branch in branch_names(&output) {
                if should_skip_merged_branch(
                    &branch,
                    &base_branch,
                    &cfg.protected_branches,
                    &current_branch,
                ) {
                    continue;
                }
                findings.push(Finding::new(
                    "CL-01",
                    Severity::Warn,
                    &format!("local branch '{branch}' is merged into {base_branch}"),
                ));
                if !dry_run && cfg.local_merged_action == "DELETE" {
                    delete_branch(
                        &mut findings,
                        &mut deleted,
                        branch.clone(),
                        &["branch", "-d", &branch],
                    );
                }
            }
        }
        Ok((rc, output)) => findings.push(Finding::new(
            "CL-01",
            Severity::Warn,
            &format!("skip local merged branch check against '{base_branch}' (rc={rc}): {output}"),
        )),
        Err(error) => findings.push(Finding::new(
            "CL-01",
            Severity::Warn,
            &format!("skip local merged branch check against '{base_branch}': {error}"),
        )),
    }

    match run_git(&["branch", "-r", "--merged", remote_base.as_str()]) {
        Ok((0, output)) => {
            for remote_ref in branch_names(&output) {
                if remote_ref.contains("->") {
                    continue;
                }
                let branch = remote_ref.strip_prefix("origin/").unwrap_or(&remote_ref);
                if should_skip_merged_branch(
                    branch,
                    &base_branch,
                    &cfg.protected_branches,
                    &current_branch,
                ) {
                    continue;
                }
                findings.push(Finding::new(
                    "CL-01",
                    Severity::Warn,
                    &format!("remote branch '{remote_ref}' is merged into {remote_base}"),
                ));
                if !dry_run && cfg.remote_merged_action == "DELETE" {
                    let label = format!("remote:{branch}");
                    delete_branch(
                        &mut findings,
                        &mut deleted,
                        label,
                        &["push", "origin", "--delete", branch],
                    );
                }
            }
        }
        Ok((rc, output)) => findings.push(Finding::new(
            "CL-01",
            Severity::Warn,
            &format!("skip remote merged branch check against '{remote_base}' (rc={rc}): {output}"),
        )),
        Err(error) => findings.push(Finding::new(
            "CL-01",
            Severity::Warn,
            &format!("skip remote merged branch check against '{remote_base}': {error}"),
        )),
    }

    if let Ok((0, output)) = run_git(&["branch", "-vv"]) {
        for line in output.lines() {
            let Some(name) = orphan_branch_name(line, &cfg.protected_branches, &current_branch)
            else {
                continue;
            };
            let branch_line = line.trim().trim_start_matches('*').trim();
            let reason = if branch_line.contains(": gone]") {
                "tracks deleted remote"
            } else {
                "has no remote tracking"
            };
            findings.push(Finding::new(
                "CL-01",
                Severity::Warn,
                &format!("local branch '{name}' {reason}"),
            ));
            if !dry_run && cfg.orphan_local_action == "DELETE" {
                delete_branch(
                    &mut findings,
                    &mut deleted,
                    name.to_string(),
                    &["branch", "-D", name],
                );
            }
        }
    }

    if let Ok((0, output)) = run_git(&["branch", "--format=%(refname:short)"]) {
        for branch in branch_names(&output) {
            if is_protected(&branch, &cfg.protected_branches, &current_branch) {
                continue;
            }
            if cfg
                .temp_branch_prefixes
                .iter()
                .any(|prefix| branch.starts_with(prefix))
            {
                findings.push(Finding::new(
                    "CL-01",
                    Severity::Warn,
                    &format!("temp branch '{branch}' matches temp prefix"),
                ));
                if !dry_run && cfg.temp_branch_action == "DELETE" {
                    delete_branch(
                        &mut findings,
                        &mut deleted,
                        branch.clone(),
                        &["branch", "-D", &branch],
                    );
                }
            }
        }
    }

    if deleted.is_empty() && findings.is_empty() {
        findings.push(Finding::new(
            "cleanup",
            Severity::Info,
            "no stale branches found",
        ));
    } else if !deleted.is_empty() {
        findings.push(Finding::new(
            "CL-01",
            Severity::Info,
            &format!("deleted {} branch(es): {deleted:?}", deleted.len()),
        ));
    }

    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_names_strip_current_and_worktree_markers() {
        let branches: Vec<_> = branch_names("* main\n+ trunk\n  feat/x\n").collect();
        assert_eq!(branches, vec!["main", "trunk", "feat/x"]);
    }

    #[test]
    fn protected_matches_exact_only_like_python() {
        let protected = vec!["main".to_string(), "release/*".to_string()];
        assert!(is_protected("main", &protected, "feat/current"));
        assert!(is_protected("feat/current", &protected, "feat/current"));
        assert!(!is_protected("release/1", &protected, "feat/current"));
    }

    #[test]
    fn orphan_branch_name_includes_gone_upstream() {
        let protected = Vec::new();
        assert_eq!(
            orphan_branch_name("feat/x abc123 [origin/feat/x: gone] msg", &protected, ""),
            Some("feat/x")
        );
        assert_eq!(
            orphan_branch_name("feat/y abc123 [origin/feat/y] msg", &protected, ""),
            None
        );
        assert_eq!(
            orphan_branch_name("feat/z abc123 msg", &protected, ""),
            Some("feat/z")
        );
    }

    #[test]
    fn branch_from_origin_head_strips_remote_prefix() {
        assert_eq!(branch_from_origin_head("origin/main\n"), Some("main"));
        assert_eq!(branch_from_origin_head("origin/master"), Some("master"));
        assert_eq!(branch_from_origin_head("main"), None);
    }

    #[test]
    fn base_branch_is_skipped_even_when_not_protected() {
        let protected = Vec::new();
        assert!(should_skip_merged_branch(
            "trunk",
            "trunk",
            &protected,
            "feat/current"
        ));
        assert!(!should_skip_merged_branch(
            "feat/old",
            "trunk",
            &protected,
            "feat/current"
        ));
    }
}
