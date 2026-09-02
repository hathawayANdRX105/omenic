//! System prompt for post-run acceptance verification.
//!
//! Used by the runner after a `Done` outcome to ask a small LLM whether
//! the assistant's last message actually satisfies the `Task.acceptance`
//! criteria. `PASS` flips the task to `Done`; anything else flips it to
//! `Failed` with the verifier's reason as the failure summary.
//!
//! This is the cheap "Done vs Done-shaped" check. The runner
//! (`crates/task/src/runner.rs`) currently trusts the model's `Done` flag
//! blindly; this prompt is the first step to making that check real.

pub const VERIFIER: &str = include_str!("../prompts/acceptance/verifier.md");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_demands_pass_or_fail_line() {
        // The runner will pattern-match on a trailing `PASS`/`FAIL:` line.
        // Make sure the prompt asks for exactly that.
        assert!(VERIFIER.contains("PASS"));
        assert!(VERIFIER.contains("FAIL"));
    }
}
