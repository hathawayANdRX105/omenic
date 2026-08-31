//! Git helpers: derive repo, current branch, find .githooks.

use std::path::PathBuf;
use std::process::Command;
use std::sync::LazyLock;

use regex::Regex;

static GITHUB_URL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"github\.com[:/]([^/]+)/([^/.\s]+)").unwrap());

/// Derive `owner/repo` from `git remote get-url origin`.
pub fn derive_repo() -> Option<String> {
    let out = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&out.stdout);
    GITHUB_URL_RE
        .captures(&url)
        .map(|c| format!("{}/{}", &c[1], &c[2]))
}

/// Current git branch name, or `None` for detached HEAD.
pub fn current_branch() -> Option<String> {
    let out = Command::new("git")
        .args(["branch", "--show-current"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if branch.is_empty() || branch == "HEAD" {
        return None;
    }
    Some(branch)
}

/// Walk up from cwd to find `.githooks/`.
pub fn find_githooks_dir() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
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

/// Read `.git/COMMIT_EDITMSG` relative to the repo root.
pub fn read_commit_editmsg() -> Option<String> {
    let root = git_root()?;
    let msg_file = root.join(".git").join("COMMIT_EDITMSG");
    std::fs::read_to_string(&msg_file).ok()
}

/// Repo root from `git rev-parse --show-toplevel`.
pub fn git_root() -> Option<PathBuf> {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if p.is_empty() {
        return None;
    }
    Some(PathBuf::from(p))
}

/// Find the open PR number for a given branch via `gh api`.
pub fn find_pr_for_branch(repo: &str, branch: &str) -> Option<u32> {
    let owner = repo.split('/').next()?;
    let path = format!("repos/{}/pulls?head={}:{}&state=open", repo, owner, branch);
    let json = crate::shared::gh_api(&path, None).ok()?;
    let prs = json.as_array()?;
    let pr = prs.first()?;
    let num = pr.get("number")?.as_u64()? as u32;
    Some(num)
}
