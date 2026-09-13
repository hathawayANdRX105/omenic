//! `subagent` — read-only parallel exploration tool.
//!
//! Public surface:
//! - [`run_subagent`] — single subagent loop, returns collected text or error.
//! - [`TaskTool`] — `tools::Tool` impl for the `task` tool (opt-in).
//! - [`config`] — limits and system prompt.
//!
//! Subagent contracts (also see crate root docs in config.rs):
//! - Read-only tool set: `read` / `grep` / `glob`. No write, edit, bash.
//! - 5-minute wall-clock cap when called via `TaskTool` (see
//!   `config::SUBAGENT_TIMEOUT_SECS`).
//! - 128KB output truncation; overflow spills to `/tmp/oi-subagent-<pid>-<id>.txt`.
//! - Drop abort: the per-subagent signal is flipped when the thread scope
//!   exits or the tool call returns, so any in-flight loop iteration sees it.
//!
//! The subagent has no persistent identity, no mailbox, no model switch. It
//! exists for one tool call and dies. The main agent must not assume state
//! across calls.

pub mod config;
pub mod parallel;
pub mod runner;
pub mod task_tool;

pub use runner::{SubagentError, SubagentEvent, run_subagent};
pub use task_tool::TaskTool;
