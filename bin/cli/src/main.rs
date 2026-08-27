//! omenic — task-driven agent orchestrator.
//!
//! Agents act as functions following Prompt → Result.

use std::process::ExitCode;

mod cli;
mod config;

fn main() -> ExitCode {
    cli::run()
}
