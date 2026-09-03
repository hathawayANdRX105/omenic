//! Tool inventory table rendered into the main agent's system prompt.
//!
//! Mirrors `oh-my-pi` `system-prompt.ts:518 buildSystemPromptToolMetadata`
//! and `system-prompt.ts:894 DEFAULT_SYSTEM_PROMPT_TOOL_NAMES`. We don't
//! have a tool registry abstraction yet, so the tool set is passed in by
//! callers (orbit / subagent / cli) rather than introspected.

use std::fmt::Write;

/// One tool entry for the prompt table.
pub struct ToolEntry {
    /// Wire name the model sees (e.g. "read", "edit", "run_bash").
    pub name: &'static str,
    /// One-line description shown in the prompt.
    pub purpose: &'static str,
}

/// Render a "## Tools" markdown table for embedding into a system prompt.
///
/// Format is deliberately plain text so the result is stable across providers.
/// The caller decides the table's neighbors (header / footer markdown) by
/// composing [`render_tools_table`] into its own fragment.
pub fn render_tools_table(tools: &[ToolEntry]) -> String {
    let mut out = String::from("## Tools\n");
    for t in tools {
        // Defensive: descriptions may include newlines; collapse to single line.
        let one_line = t.purpose.replace('\n', " ");
        let _ = writeln!(out, "- **{}**: {}", t.name, one_line);
    }
    out
}
