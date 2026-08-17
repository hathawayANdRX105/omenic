//! omenic — task-driven agent orchestrator.
//!
//! Agents act as functions following Prompt → Result.

use std::process::ExitCode;

fn main() -> ExitCode {
    oi_core::cli::run()
}
