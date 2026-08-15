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
    /// Run review checks
    Review,
    /// Run audit
    Audit,
    /// Validate issues
    Issue,
    /// Validate pull requests
    Pr,
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init
        | Commands::PreCommit
        | Commands::PrePush
        | Commands::Merge
        | Commands::Review
        | Commands::Audit
        | Commands::Issue
        | Commands::Pr => {
            println!("not implemented yet");
        }
    }
}
