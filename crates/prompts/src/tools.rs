//! Tool inventory table rendered into the main agent's system prompt.
//!
//! Mirrors `oh-my-pi` `system-prompt.ts:518 buildSystemPromptToolMetadata`
//! and `system-prompt.ts:894 DEFAULT_SYSTEM_PROMPT_TOOL_NAMES`. We don't
//! have a tool registry abstraction yet, so the tool set is passed in by
//! callers (orbit / subagent / cli) rather than introspected.

use std::fmt::Write;

/// One tool entry for the prompt table. Use the borrowed-string
/// [`render_tools_table_pairs`] variant when the tool data is owned
/// elsewhere (e.g. `ToolDef` from the adaptor) and shouldn't be cloned.
pub struct ToolEntry {
    /// Wire name the model sees (e.g. "read", "edit", "run_bash").
    pub name: &'static str,
    /// One-line description shown in the prompt.
    pub purpose: &'static str,
}

/// Render a "## Tools" markdown table for embedding into a system prompt.
pub fn render_tools_table(tools: &[ToolEntry]) -> String {
    render_tools_table_pairs(tools.iter().map(|t| (t.name, t.purpose)))
}

/// Borrowed-string version of [`render_tools_table`]. Callers with owned
/// strings (e.g. `ToolDef` from the adaptor) avoid an extra clone by
/// passing `(name, purpose)` references directly.
pub fn render_tools_table_pairs<'a, I>(tools: I) -> String
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let mut out = String::from("## Tools\n");
    for (name, purpose) in tools {
        // Defensive: descriptions may include newlines; collapse to single line.
        let one_line = purpose.replace('\n', " ");
        let _ = writeln!(out, "- **{}**: {}", name, one_line);
    }
    out
}
