//! omenic — task-driven agent orchestrator.
//!
//! Agents act as functions following Prompt → Result.
//! Module layout per spike/mvp-design § 6: single crate, one module per concern.

mod cli;
mod config;
mod graph;
mod rpc;
mod runner;
mod store;
mod task;
mod worker;

fn main() {
    // CLI entry; command routing lands in `cli` (M1.8).
    cli::run();
}
