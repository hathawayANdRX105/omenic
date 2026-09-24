//! Agent role prompts, copied verbatim from
//! `oh-my-pi`'s `packages/coding-agent/src/prompts/agents/`.
//!
//! Each constant is the entire contents of a `.md` file under
//! `crates/infra/prompts/prompts/agents/`. The whole file is the prompt;
//! omenic sends it verbatim to the LLM with no concatenation. File
//! names match omp exactly; see the per-role description in each
//! `.md`'s frontmatter (or body, for `task.md` which has none).
//!
//! omenic currently uses only [`TASK`] (via `crates/agent/orbit`) and
//! [`SCOUT`] (planned for `crates/agent/subagent/src/config.rs`). The
//! other 6 roles are kept for future wiring — omenic will adopt them
//! when it gains the corresponding tooling.

/// Agent profile `designer.md` (verbatim from omp).
pub const DESIGNER: &str = include_str!("../prompts/agents/designer.md");

/// Agent profile `frontmatter.md` (verbatim from omp).
pub const FRONTMATTER: &str = include_str!("../prompts/agents/frontmatter.md");

/// Agent profile `init.md` (verbatim from omp).
pub const INIT: &str = include_str!("../prompts/agents/init.md");

/// Agent profile `librarian.md` (verbatim from omp).
pub const LIBRARIAN: &str = include_str!("../prompts/agents/librarian.md");

/// Agent profile `reviewer.md` (verbatim from omp).
pub const REVIEWER: &str = include_str!("../prompts/agents/reviewer.md");

/// Agent profile `scout.md` (verbatim from omp).
pub const SCOUT: &str = include_str!("../prompts/agents/scout.md");

/// Agent profile `security-reviewer.md` (verbatim from omp).
pub const SECURITY_REVIEWER: &str = include_str!("../prompts/agents/security-reviewer.md");

/// Agent profile `task.md` (verbatim from omp).
pub const TASK: &str = include_str!("../prompts/agents/task.md");
