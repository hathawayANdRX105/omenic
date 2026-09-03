//! System prompt for context compaction.
//!
//! Used by `orbit::compact_context` (lib.rs:252-267) to ask the LLM to
//! summarize the dropped window into a single assistant message that
//! replaces the cut in `context.messages`.
//!
//! Mirrors `zerostack` `src/agent/prompt.rs::COMPACTION_PROMPT` and
//! `jcode` `crates/jcode-app-core/src/agent/compaction.rs`.

pub const SUMMARIZER: &str = include_str!("../prompts/compaction/summarizer.md");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_requires_structured_output() {
        // Compaction must produce a stable shape so `ContextLog::load`
        // can replay it later. Lock that in via the prompt.
        let lower = SUMMARIZER.to_ascii_lowercase();
        assert!(lower.contains("goal"));
        assert!(lower.contains("progress"));
        assert!(lower.contains("next"));
    }
}
