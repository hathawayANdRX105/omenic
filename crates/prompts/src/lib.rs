//! System prompts for omenic agents.
//!
//! Mirrors `oh-my-pi`'s `packages/coding-agent/src/prompts/` layout:
//! ordered `.md` fragments under `crates/prompts/prompts/<role>/` are
//! assembled at build time via [`include_str!`], then exposed as Rust
//! constants for callers (orbit, subagent, compaction).
//!
//! See also:
//! - `oh-my-pi`: `packages/coding-agent/src/prompts/system/*` + `system-prompt.ts:666 buildSystemPrompt`
//! - `zerostack`: `src/agent/prompt.rs` (`SYSTEM_PROMPT` constant)
//! - `jcode`:    `crates/jcode-app-core/src/agent/prompting.rs`
//!
//! ## Roles
//!
//! | Module | Role | Notes |
//! |---|---|---|
//! | [`main_agent`] | Top-level orchestrator agent | Reads `Context.system_prompt`; given tool set |
//! | [`subagent`] | Read-only explorer | Existing `crates/subagent` `task` tool reuses this |
//! | [`compaction`] | Context summarizer | Injected by `orbit::compact_context` |
//! | [`acceptance`] | Done/Failed verifier | Post-run checklist against `Task.acceptance` |
//!
//! Every prompt is a single `&'static str` constant — no `format!` cost, no I/O,
//! no async — so they can be used from any sync context (subagent tool dispatch,
//! orbit loop, CLI startup).

pub mod acceptance;
pub mod compaction;
pub mod main_agent;
pub mod subagent;

pub mod tools;
