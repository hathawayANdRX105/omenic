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
    Init {
        /// Uninstall gate instead of installing
        #[arg(long)]
        uninstall: bool,
    },
    /// Run pre-commit hooks
    PreCommit,
    /// Run pre-push hooks
    PrePush,
    /// Run merge checks: gate merge <owner/repo> <pr_number> [--dry-run]
    Merge {
        /// Arguments: <owner/repo> <pr_number> [--dry-run]
        args: Vec<String>,
    },
    /// Run review checks: gate review [owner/repo] [--post]
    Review {
        /// Arguments: [owner/repo] [--post]
        args: Vec<String>,
    },
    /// Run audit: gate audit <owner/repo> [--issues=N,M] [--recent=N] [--limit=N]
    Audit {
        /// Arguments: <owner/repo> [--issues=N,M] [--recent=N] [--limit=N]
        #[arg(allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Validate an issue: gate issue <owner/repo> <number>
    Issue {
        /// Arguments: <owner/repo> <number>
        args: Vec<String>,
    },
    /// Validate a pull request: gate pr <owner/repo> <number>
    Pr {
        /// Arguments: <owner/repo> <number>
        args: Vec<String>,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init { uninstall } => {
            if uninstall {
                std::process::exit(match gate_core::tools::init::uninstall() {
                    Ok(()) => 0,
                    Err(e) => {
                        eprintln!("gate init --uninstall failed: {:#}", e);
                        1
                    }
                });
            } else {
                std::process::exit(match gate_core::tools::init::install() {
                    Ok(()) => 0,
                    Err(e) => {
                        eprintln!("gate init failed: {:#}", e);
                        1
                    }
                });
            }
        }
        Commands::PreCommit => {
            std::process::exit(gate_core::tools::pre_commit::run());
        }
        Commands::PrePush => {
            std::process::exit(gate_core::tools::pre_push::run());
        }
        Commands::Merge { args } => {
            std::process::exit(gate_core::tools::merge::run(&args));
        }
        Commands::Review { args } => {
            std::process::exit(gate_core::tools::review::run(&args));
        }
        Commands::Audit { args } => {
            std::process::exit(gate_core::tools::audit::run(&args));
        }
        Commands::Issue { args } => {
            if args.len() < 2 {
                eprintln!("Usage: gate issue <owner/repo> <number>");
                std::process::exit(2);
            }
            let repo = &args[0];
            let num: u32 = args[1].parse().unwrap_or_else(|_| {
                eprintln!("invalid issue number: {}", args[1]);
                std::process::exit(2);
            });
            std::process::exit(run_issue_validation(repo, num));
        }
        Commands::Pr { args } => {
            if args.len() < 2 {
                eprintln!("Usage: gate pr <owner/repo> <number>");
                std::process::exit(2);
            }
            let repo = &args[0];
            let num: u32 = args[1].parse().unwrap_or_else(|_| {
                eprintln!("invalid PR number: {}", args[1]);
                std::process::exit(2);
            });
            std::process::exit(run_pr_validation(repo, num));
        }
    }
}

/// Validate a single issue against IS-* rules.
fn run_issue_validation(repo: &str, num: u32) -> i32 {
    use gate_core::shared::{gh_api, print_findings, exit_code};
    match gh_api(&format!("repos/{}/issues/{}", repo, num), None) {
        Ok(data) => {
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
            let findings = gate_core::rules::issues::check_content(title, body, &labels, "sub", state);
            print_findings(&findings);
            exit_code(&findings)
        }
        Err(e) => {
            eprintln!("could not fetch issue #{}: {}", num, e);
            1
        }
    }
}

/// Validate a single PR against PR-* rules.
fn run_pr_validation(repo: &str, num: u32) -> i32 {
    use gate_core::shared::{gh_api, print_findings, exit_code};
    match gh_api(&format!("repos/{}/pulls/{}", repo, num), None) {
        Ok(pr) => {
            let title = pr.get("title").and_then(|t| t.as_str()).unwrap_or("");
            let body = pr.get("body").and_then(|b| b.as_str()).unwrap_or("");
            let state = pr.get("state").and_then(|s| s.as_str()).unwrap_or("open");
            let head = pr
                .get("head")
                .and_then(|h| h.get("ref"))
                .and_then(|r| r.as_str())
                .unwrap_or("");
            let draft = pr.get("draft").and_then(|d| d.as_bool()).unwrap_or(false);
            let labels: Vec<&str> = pr
                .get("labels")
                .and_then(|l| l.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|l| l.get("name").and_then(|n| n.as_str()))
                        .collect()
                })
                .unwrap_or_default();
            let findings =
                gate_core::rules::pull_requests::check_content(title, body, &labels, head, state, draft, None);
            print_findings(&findings);
            exit_code(&findings)
        }
        Err(e) => {
            eprintln!("could not fetch PR #{}: {}", num, e);
            1
        }
    }
}
