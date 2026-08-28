use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "gate", version, about = "omenic gate CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize gate configuration
    Init,
    /// Run pre-commit hooks
    PreCommit,
    /// Run pre-push hooks
    PrePush,
    /// Run merge checks: `gate merge <owner/repo> <pr_number> [--dry-run]`
    Merge(MergeArgs),
    /// Run CRG + ocr code review
    Review(ReviewArgs),
    /// Audit issues/PRs for checkbox & linkage compliance
    Audit(AuditArgs),
    /// Validate issues
    Issue,
    /// Validate issues
    Pr,
}

#[derive(clap::Args)]
struct ReviewArgs {
    /// Post results as a PR conversation comment
    #[arg(long)]
    post: bool,
    /// Post inline review comments on the PR diff
    #[arg(long = "post-inline")]
    post_inline: bool,
    /// PR number to post to (auto-detected if omitted)
    #[arg(long)]
    pr: Option<u64>,
}

#[derive(clap::Args)]
struct MergeArgs {
    /// owner/repo
    repo: String,
    /// PR number
    pr: u32,
    /// Plan only, no squash
    #[arg(long)]
    dry_run: bool,
}

#[derive(clap::Args)]
struct AuditArgs {
    /// owner/repo (defaults to git remote origin)
    repo: Option<String>,
    /// Scan issues/PRs created in the last N days
    #[arg(long)]
    recent: Option<u32>,
    /// Limit number of items to scan (0 = unlimited)
    #[arg(long, default_value = "0")]
    limit: u32,
    /// Number of concurrent workers
    #[arg(long, default_value = "5")]
    workers: u32,
    /// Specific issue/PR numbers to check
    #[arg(long)]
    issues: Option<String>,
}

fn main() -> ExitCode {
    // gh-mode: installed as ~/.local/bin/gh → intercept issue/pr commands
    if let Some(arg0) = std::env::args().next() {
        let base = std::path::Path::new(&arg0)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if base == "gh" || base == "gh.exe" {
            let args: Vec<String> = std::env::args().skip(1).collect();
            let rc = spec::gate::tools::gh_wrap::dispatch(&args);
            return ExitCode::from(rc as u8);
        }
    }

    let cli = Cli::parse();
    match cli.command {
        Commands::Init => match spec::gate::tools::init::install() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("gate init 失败: {e}");
                ExitCode::FAILURE
            }
        },
        Commands::PreCommit => ExitCode::from(spec::gate::tools::pre_commit::run() as u8),
        Commands::PrePush => ExitCode::from(spec::gate::tools::pre_push::run() as u8),
        Commands::Merge(args) => {
            let mut arg_vec = vec![args.repo, args.pr.to_string()];
            if args.dry_run {
                arg_vec.push("--dry-run".to_string());
            }
            ExitCode::from(spec::gate::tools::merge::run(&arg_vec) as u8)
        }
        Commands::Issue => {
            ExitCode::from(spec::gate::tools::gh_wrap::intercept_issue_create(&[]) as u8)
        }
        Commands::Pr => ExitCode::from(spec::gate::tools::gh_wrap::intercept_pr_create(&[]) as u8),
        Commands::Review(args) => {
            let args_vec: Vec<String> = build_review_args(&args);
            let rc = spec::gate::tools::review::run(&args_vec);
            ExitCode::from(rc as u8)
        }
        Commands::Audit(args) => {
            let args_vec: Vec<String> = build_audit_args(&args);
            let rc = spec::gate::tools::audit::run(&args_vec);
            ExitCode::from(rc as u8)
        }
    }
}

fn build_review_args(args: &ReviewArgs) -> Vec<String> {
    let mut vec = Vec::new();
    if args.post {
        vec.push("--post".to_string());
    }
    if args.post_inline {
        vec.push("--post-inline".to_string());
    }
    if let Some(pr) = args.pr {
        vec.push("--pr".to_string());
        vec.push(pr.to_string());
    }
    vec
}

fn build_audit_args(args: &AuditArgs) -> Vec<String> {
    let mut vec = Vec::new();
    let repo = args
        .repo
        .clone()
        .unwrap_or_else(spec::gate::tools::audit::derive_repo);
    if repo.is_empty() {
        eprintln!("无法确定 repo (git remote get-url origin 失败)");
        return vec![];
    }
    vec.push(repo);
    if let Some(days) = args.recent {
        vec.push(format!("--recent={days}"));
    }
    if args.limit > 0 {
        vec.push(format!("--limit={}", args.limit));
    }
    if args.workers != 5 {
        vec.push(format!("--workers={}", args.workers));
    }
    if let Some(issues) = &args.issues {
        vec.push(format!("--issues={issues}"));
    }
    vec
}

#[cfg(test)]
mod tests {
    use spec::gate::shared::load_yaml;

    #[test]
    fn loads_real_spec_and_counts_required_headings() {
        let path = "/home/hathaway/projects/omenic/.githooks/spec/github_issues.yaml";
        let v = load_yaml(path).expect("spec yaml must parse");
        let headings = v
            .get("required_headings")
            .expect("required_headings key present")
            .as_sequence()
            .expect("required_headings is a sequence");
        assert_eq!(headings.len(), 6, "expected 6 required headings");
        let names: Vec<&str> = headings.iter().filter_map(|h| h.as_str()).collect();
        assert!(names.contains(&"Goal"));
        assert!(names.contains(&"Out of scope"));
    }
}
