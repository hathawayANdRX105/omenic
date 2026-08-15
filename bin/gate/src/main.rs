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
    /// Run merge checks
    Merge,
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
struct AuditArgs {
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
            let rc = gate_core::tools::gh_wrap::dispatch(&args);
            return ExitCode::from(rc as u8);
        }
    }

    let cli = Cli::parse();
    match cli.command {
        Commands::Init => {
            match gate_core::tools::init::install() {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("gate init 失败: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Commands::PreCommit => ExitCode::from(gate_core::tools::pre_commit::run() as u8),
        Commands::PrePush => ExitCode::from(gate_core::tools::pre_push::run() as u8),
        Commands::Merge => ExitCode::from(gate_core::tools::merge::run(&[]) as u8),
        Commands::Issue => ExitCode::from(gate_core::tools::gh_wrap::intercept_issue_create(&[]) as u8),
        Commands::Pr => ExitCode::from(gate_core::tools::gh_wrap::intercept_pr_create(&[]) as u8),
        Commands::Review(args) => {
            let args_vec: Vec<String> = build_review_args(&args);
            let rc = gate_core::tools::review::run(&args_vec);
            ExitCode::from(rc as u8)
        }
        Commands::Audit(args) => {
            let args_vec: Vec<String> = build_audit_args(&args);
            let rc = gate_core::tools::audit::run(&args_vec);
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
    let repo = gate_core::tools::audit::derive_repo();
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
