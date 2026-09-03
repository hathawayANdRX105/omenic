//! Agent role prompts, lifted from `oh-my-pi`.
//!
//! Each constant is the entire contents of a `.md` file under
//! `crates/prompts/prompts/agents/`. The whole file is the prompt —
//! frontmatter included, exactly as omp sends it to its LLM.

/// Main agent profile, copied from `oh-my-pi`'s `prompts/agents/task.md`.
///
/// Tools: full access. Spawns `scout` subagents for cross-file work.
pub const MAIN: &str = include_str!("../prompts/agents/main.md");

/// Read-only explorer profile, copied from
/// `oh-my-pi`'s `prompts/agents/scout.md`.
///
/// Tools: `read`, `grep`, `glob` only. Must not write, edit, or run
/// state-changing commands.
pub const SCOUT: &str = include_str!("../prompts/agents/scout.md");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_prompt_has_frontmatter_and_directives() {
        // omp-style profile: starts with `---`, has `<directives>` block.
        assert!(
            MAIN.starts_with("---\n"),
            "main.md must start with YAML frontmatter"
        );
        assert!(
            MAIN.contains("<directives>"),
            "main.md must have <directives> block"
        );
        assert!(
            MAIN.contains("<critical>"),
            "main.md must have <critical> block"
        );
    }

    #[test]
    fn scout_prompt_declares_read_only() {
        // Mirror the assertion the previous subagent.rs test made: the
        // scout profile must explicitly declare a read-only stance.
        let lower = SCOUT.to_ascii_lowercase();
        assert!(
            lower.contains("read-only") || lower.contains("read only"),
            "scout.md must declare a read-only stance"
        );
    }

    #[test]
    fn scout_prompt_lists_three_tools_in_frontmatter() {
        // The `tools:` line in the YAML frontmatter must list exactly
        // read, grep, glob (the tools omenic's subagent exposes).
        let tools_line = SCOUT
            .lines()
            .find(|l| l.starts_with("tools:"))
            .expect("scout.md must have a `tools:` frontmatter line");
        let body = tools_line.trim_start_matches("tools:").trim();
        let listed: Vec<&str> = body
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        assert_eq!(listed, vec!["read", "grep", "glob"]);
    }
}
