//! oi-core: task-driven agent orchestrator core library.
//!
//! Layout by theme:
//! - `task` — task model: types (`task`), persistence (`store`), deps (`graph`)
//! - `workflow` — content definition: templates + spec tables
//! - `runtime` — agent kernel: LLM stream (`llm`), invariant loop (`agent`),
//!   builtin tools (`tools`)
//! - `orchestration` — transport (`omp_rpc`), process (`agent_process`),
//!   orchestration (`run_flow`)
//! - crate root — entry (`cli`), settings (`config`)

pub mod cli;
pub mod config;
pub mod orchestration;
pub mod runtime;
pub mod task;
pub mod workflow;
