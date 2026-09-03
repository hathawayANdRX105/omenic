//! System prompt for the top-level orchestrator agent.
//!
//! Assembled at build time from `.md` fragments under
//! `crates/prompts/prompts/main/`. Callers pass the tool set; the
//! rendered prompt is what `Context.system_prompt` carries into
//! `orbit::run_agent_streaming`.
//!
//! Mirrors `oh-my-pi` `packages/coding-agent/src/prompts/system/*.md` and
//! `zerostack` `src/agent/prompt.rs::SYSTEM_PROMPT`. We keep the
//! fragments external so users can override them via a project-level
//! `SYSTEM.md` drop-in later (see `oh-my-pi` `loadSystemPromptFiles`).

use crate::tools::{ToolEntry, render_tools_table};

const CORE: &str = include_str!("../prompts/main/core.md");
const TASK_DECOMPOSITION: &str = include_str!("../prompts/main/task-decomposition.md");
const ACCEPTANCE_CRITERIA: &str = include_str!("../prompts/main/acceptance-criteria.md");
const RULES: &str = include_str!("../prompts/main/rules.md");

/// Build the main agent's system prompt for the given tool set.
///
/// Concatenation order is fixed: `core → tools → decomposition → rules →
/// acceptance`. Order matters — the model weights the last block of the
/// system prompt most heavily for `Done`/`Failed` decisions, so the
/// acceptance criteria sit at the end.
pub fn build(tools: &[ToolEntry]) -> String {
    let mut out = String::with_capacity(2048);
    out.push_str(CORE);
    out.push_str("\n\n");
    out.push_str(&render_tools_table(tools));
    out.push('\n');
    out.push_str(TASK_DECOMPOSITION);
    out.push_str("\n\n");
    out.push_str(RULES);
    out.push_str("\n\n");
    out.push_str(ACCEPTANCE_CRITERIA);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_includes_every_fragment() {
        let tools = vec![
            ToolEntry {
                name: "read",
                purpose: "Read files",
            },
            ToolEntry {
                name: "edit",
                purpose: "Edit files",
            },
        ];
        let p = build(&tools);
        // Headings are '# Section' (level 1) per the .md fragments; check
        // for the section anchor, not the markdown level.
        assert!(
            p.contains("# Acceptance Criteria"),
            "missing acceptance section: {p}"
        );
        assert!(p.contains("# Task Decomposition"));
        assert!(p.contains("# Rules"));
        assert!(p.contains("- **read**"));
        assert!(p.contains("- **edit**"));
    }

    #[test]
    fn build_orders_acceptance_last() {
        // Acceptance at the end: matters for the model's "Done" weight.
        let p = build(&[]);
        let acceptance_pos = p
            .find("# Acceptance Criteria")
            .expect("acceptance section present");
        let rules_pos = p.find("# Rules").expect("rules section present");
        let decomp_pos = p
            .find("# Task Decomposition")
            .expect("decomp section present");
        assert!(acceptance_pos > rules_pos);
        assert!(rules_pos > decomp_pos);
    }
}
