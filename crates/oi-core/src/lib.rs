//! oi-core: task-driven agent orchestrator core library.
//!
//! Modules:
//! - `task` — core types (Task, TaskKind, TaskStatus)
//! - `store` — JSONL append-only task store
//! - `graph` — dependency graph (anti-cycle, ready/blocked)
//! - `config` — configuration (omp path, data dir, model)
//! - `template` — YAML phase/step templates + apply
//! - `spec` — spec tables (issue/epic/pr/review)
//! - `rpc` — omp RPC client (stdio JSONL)
//! - `worker` — worker lifecycle (spawn/steer/abort/events)
//! - `runner` — orchestration run flow
//! - `cli` — CLI command layer

pub mod cli;
pub mod config;
pub mod graph;
pub mod harness;
pub mod rpc;
pub mod runner;
pub mod spec;
pub mod store;
pub mod task;
pub mod template;
pub mod worker;
