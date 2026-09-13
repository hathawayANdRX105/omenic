//! System prompts for omenic agents, copied verbatim from
//! `oh-my-pi`'s `packages/coding-agent/src/prompts/`.
//!
//! Layout mirrors omp directly:
//!
//! ```text
//! oh-my-pi                                  omenic
//! packages/coding-agent/src/prompts/        crates/prompts/prompts/
//! ├── agents/                               ├── agents/
//! │   ├── task.md        ──►               │   ├── task.md
//! │   ├── scout.md       ──►               │   ├── scout.md
//! │   ├── librarian.md   ──►               │   ├── librarian.md
//! │   ├── reviewer.md    ──►               │   ├── reviewer.md
//! │   ├── designer.md    ──►               │   ├── designer.md
//! │   ├── init.md        ──►               │   ├── init.md
//! │   ├── security-reviewer.md ──►         │   ├── security-reviewer.md
//! │   └── frontmatter.md ──►               │   └── frontmatter.md
//! └── system/                               └── system/
//!     ├── *.md (73 fragments)  ──►             ├── *.md (73 fragments)
//!     └── personalities/           ──►         └── personalities/
//! ```
//!
//! Each `.md` file is exposed as a `&'static str` constant via
//! `include_str!`. There is no concatenation, no tool-table generation,
//! no frontmatter stripping — the whole file is the prompt, exactly as
//! omp sends it. Callers compose fragments at the role layer (see
//! `crates/orbit` for the only current consumer: `agents::TASK`).
//!
//! The `crates/prompts/prompts/system/` files (73 fragments including the
//! 4 `personalities/`) are omenic's fragment library, available as
//! `prompts::system::ACTIVE_REPO_CONTEXT` / `::COMPUTER_SAFETY` / etc.
//! omenic does not yet have an omp-equivalent `buildSystemPrompt` prep
//! step (no per-fragment conditional gating), so callers wanting to
//! inject fragments must compose them by hand. Future PRs may add a
//! prep step that mirrors `system-prompt.ts:666`.

pub mod agents;
pub mod system;
