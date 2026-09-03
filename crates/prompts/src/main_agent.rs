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

use crate::tools::{ToolEntry, render_tools_table_pairs};

const CORE: &str = include_str!("../prompts/main/core.md");
const TASK_DECOMPOSITION: &str = include_str!("../prompts/main/task-decomposition.md");
const ACCEPTANCE_CRITERIA: &str = include_str!("../prompts/main/acceptance-criteria.md");
const RULES: &str = include_str!("../prompts/main/rules.md");

/// Build the main agent's system prompt for the given tool set.
pub fn build(tools: &[ToolEntry]) -> String {
    build_pairs(tools.iter().map(|t| (t.name, t.purpose)))
}

/// Same assembly as [`build`], but takes borrowed `(name, purpose)` pairs
/// so callers with owned strings (e.g. `ToolDef` from the adaptor) don't
/// have to clone into `ToolEntry` first.
pub fn build_pairs<'a, I>(tools: I) -> String
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let mut out = String::with_capacity(2048);
    out.push_str(CORE);
    out.push_str("\n\n");
    out.push_str(&render_tools_table_pairs(tools));
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

    #[test]
    fn build_pairs_matches_build_for_borrowed_strings() {
        // Owning callers (e.g. orbit with `ToolDef`) should get the same
        // output as static `ToolEntry` callers when the content matches.
        let owned: Vec<(&str, &str)> = vec![("read", "Read files"), ("edit", "Edit files")];
        let owned_rendered = build_pairs(owned);
        let static_rendered = build(&[
            ToolEntry {
                name: "read",
                purpose: "Read files",
            },
            ToolEntry {
                name: "edit",
                purpose: "Edit files",
            },
        ]);
        assert_eq!(owned_rendered, static_rendered);
    }
}
