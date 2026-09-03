//! Agent role prompts, copied verbatim from
//! `oh-my-pi`'s `packages/coding-agent/src/prompts/agents/`.
//!
//! Each constant is the entire contents of a `.md` file under
//! `crates/prompts/prompts/agents/`. The whole file is the prompt;
//! omenic sends it verbatim to the LLM with no concatenation. File
//! names match omp exactly; see the per-role description in each
//! `.md`'s frontmatter (or body, for `task.md` which has none).
//!
//! omenic currently uses only [`TASK`] (via `crates/orbit`) and
//! [`SCOUT`] (planned for `crates/subagent/src/config.rs:19`). The
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn designer_has_named_frontmatter() {
        assert!(
            DESIGNER.starts_with("---\n"),
            "designer.md must start with YAML frontmatter"
        );
        assert!(
            DESIGNER.contains("\nname: designer\n") || DESIGNER.contains("\nname: \"designer\""),
            "designer.md frontmatter must declare name: designer"
        );
    }

    #[test]
    fn frontmatter_is_template() {
        assert!(
            FRONTMATTER.contains("{{"),
            "frontmatter.md is a Mustache template"
        );
    }

    #[test]
    fn init_has_named_frontmatter() {
        assert!(
            INIT.starts_with("---\n"),
            "init.md must start with YAML frontmatter"
        );
        assert!(
            INIT.contains("\nname: init\n") || INIT.contains("\nname: \"init\""),
            "init.md frontmatter must declare name: init"
        );
    }

    #[test]
    fn librarian_has_named_frontmatter() {
        assert!(
            LIBRARIAN.starts_with("---\n"),
            "librarian.md must start with YAML frontmatter"
        );
        assert!(
            LIBRARIAN.contains("\nname: librarian\n")
                || LIBRARIAN.contains("\nname: \"librarian\""),
            "librarian.md frontmatter must declare name: librarian"
        );
    }

    #[test]
    fn reviewer_has_named_frontmatter() {
        assert!(
            REVIEWER.starts_with("---\n"),
            "reviewer.md must start with YAML frontmatter"
        );
        assert!(
            REVIEWER.contains("\nname: reviewer\n") || REVIEWER.contains("\nname: \"reviewer\""),
            "reviewer.md frontmatter must declare name: reviewer"
        );
    }

    #[test]
    fn scout_has_named_frontmatter() {
        assert!(
            SCOUT.starts_with("---\n"),
            "scout.md must start with YAML frontmatter"
        );
        assert!(
            SCOUT.contains("\nname: scout\n") || SCOUT.contains("\nname: \"scout\""),
            "scout.md frontmatter must declare name: scout"
        );
    }

    #[test]
    fn security_reviewer_has_named_frontmatter() {
        assert!(
            SECURITY_REVIEWER.starts_with("---\n"),
            "security-reviewer.md must start with YAML frontmatter"
        );
        assert!(
            SECURITY_REVIEWER.contains("\nname: security-reviewer\n")
                || SECURITY_REVIEWER.contains("\nname: \"security-reviewer\""),
            "security-reviewer.md frontmatter must declare name: security-reviewer"
        );
    }

    #[test]
    fn task_has_directives_block() {
        assert!(
            TASK.contains("<directives>"),
            "task.md must have <directives> block"
        );
        assert!(
            TASK.starts_with("Worker agent:"),
            "task.md must start with the omp header"
        );
    }
}
