//! Branch cleanup: CL-01 stale branch detection and removal.

use std::path::Path;

use crate::shared::{Finding, Severity, load_yaml, run_external};

#[derive(Debug, Clone)]
struct CleanupConfig {
    protected_branches: Vec<String>,
    local_merged_action: String,
    remote_merged_action: String,
    orphan_local_action: String,
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

fn branch_names(output: &str) -> impl Iterator<Item = String> + '_ {
    output
        .lines()
        .map(|line| line.trim().trim_start_matches('*').trim().to_string())
        .filter(|line| !line.is_empty())
}

fn is_protected(branch: &str, protected: &[String], current: &str) -> bool {
    branch == current || protected.iter().any(|p| p == branch)
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
    let branch_line = line.trim().trim_start_matches('*').trim();
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

pub fn run(dry_run: bool) -> Vec<Finding> {
    let githooks = crate::tools::git::find_githooks_dir()
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

    let mut findings = Vec::new();
    let mut deleted = Vec::new();

    if let Ok((0, output)) = run_git(&["branch", "--merged", "main"]) {
        for branch in branch_names(&output) {
            if is_protected(&branch, &cfg.protected_branches, &current_branch) {
                continue;
            }
            findings.push(Finding::new(
                "CL-01",
                Severity::Warn,
                &format!("local branch '{branch}' is merged into main"),
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

    if let Ok((0, output)) = run_git(&["branch", "-r", "--merged", "origin/main"]) {
        for remote_ref in branch_names(&output) {
            if remote_ref.contains("->") {
                continue;
            }
            let branch = remote_ref.strip_prefix("origin/").unwrap_or(&remote_ref);
            if is_protected(branch, &cfg.protected_branches, &current_branch) {
                continue;
            }
            findings.push(Finding::new(
                "CL-01",
                Severity::Warn,
                &format!("remote branch '{remote_ref}' is merged into origin/main"),
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
                if !dry_run {
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
    fn branch_names_strip_current_marker() {
        let branches: Vec<_> = branch_names("* main\n  feat/x\n").collect();
        assert_eq!(branches, vec!["main", "feat/x"]);
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
}
