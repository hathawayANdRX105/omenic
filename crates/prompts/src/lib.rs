//! System prompts for omenic agents, verbatim from
//! `oh-my-pi`'s `packages/coding-agent/src/prompts/agents/`.
//!
//! Each `.md` under `crates/prompts/prompts/agents/<role>.md` is a
//! self-contained agent profile: YAML frontmatter (`name`, `description`,
//! `tools`) followed by a body of `<directives>`, `<procedure>`, `<critical>`
//! blocks. The whole file is the prompt; nothing is concatenated.
//!
//! Layout mirrors `oh-my-pi` directly:
//!
//! ```text
//! oh-my-pi                                  omenic
//! prompts/agents/task.md          ──►      prompts/agents/main.md
//! prompts/agents/scout.md         ──►      prompts/agents/scout.md
//! ```
//!
//! omenic currently has only two roles: `main` (full tool set) and `scout`
//! (read-only). Other omp roles (`librarian`, `reviewer`, `designer`, etc.)
//! are not implemented in omenic and not imported here.
//!
//! The frontmatter `tools:` line is preserved verbatim; consumers that want
//! to render a tool table should split it themselves — see
//! [`tools::render_tools_table`].

pub mod agents;
pub mod tools;
