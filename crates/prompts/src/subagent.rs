//! System prompt for the read-only subagent.
//!
//! Replaces the inline string in
//! `crates/subagent/src/config.rs::SUBAGENT_SYSTEM_PROMPT`. Kept here so
//! the prompt is editable as a `.md` fragment and shared with the
//! compaction/acceptance vocabulary.

pub const READ_ONLY_EXPLORER: &str = include_str!("../prompts/subagent/readonly-explorer.md");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_forbids_writes() {
        // The subagent prompt must declare a write prohibition. The
        // structural signal is the explicit "read-only" / "do not write"
        // claim, not the absence of the words "write" / "edit" (which
        // appear in the prompt in negated form).
        let lower = READ_ONLY_EXPLORER.to_ascii_lowercase();
        assert!(
            lower.contains("read-only") || lower.contains("do not write"),
            "subagent prompt must declare a read-only / no-write stance",
        );
    }
}
