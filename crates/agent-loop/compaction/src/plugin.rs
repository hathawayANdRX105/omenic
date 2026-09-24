//! The compaction plugin: provides the default char-budget policy as the
//! `harness.compaction` service.

use plugin::{DshPlugin, PluginContext};

use crate::policy::CharBudgetPolicy;

/// Publishes [`CharBudgetPolicy`] under `harness.compaction`. The default
/// instance carries [`crate::summarize::NoopSummarizer`] — compaction stays
/// the keep-original safe path until a host resolves the service, wraps it
/// with its own LLM summarizer (e.g. orbit's `LlmBackend` bridge), and
/// re-provides it.
pub struct CompactionPlugin;

impl DshPlugin for CompactionPlugin {
    fn name(&self) -> &str {
        "harness-compaction"
    }

    fn register(&self, ctx: &mut PluginContext<'_>) {
        ctx.provide("harness.compaction", CharBudgetPolicy::default());
    }
}
