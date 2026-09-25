//! Prompt assembly pipeline for the omenic agent harness.
//!
//! Reference: `omenic infra/prompts` (`include_str!` fragment composition)
//! and `dsh core/session` system-prompt prep (`system-prompt.ts:666`).
//!
//! This crate abstracts template rendering so that prompt composition
//! is decoupled from the loop engine (`omenic-harness-runtime`).

use protocol::Message;

/// Input to a prompt renderer: the conversation context in which
/// fragments are composed.
#[derive(Debug, Clone, Default)]
pub struct PromptInput {
    /// The base system prompt text.
    pub system: String,
    /// The user turn text.
    pub user: String,
    /// Prior conversation messages, serialized as JSON for template access.
    pub history: Vec<Message>,
}

/// A named prompt template fragment, mirroring `prompts::system::*`
/// constants in the omenic `infra/prompts` crate.
#[derive(Debug, Clone)]
pub struct PromptTemplate {
    pub name: String,
    /// Static fragment text (mirrors `include_str!` in `infra/prompts`).
    pub body: String,
}

impl PromptTemplate {
    pub fn new(name: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            body: body.into(),
        }
    }
}

/// Renders a prompt from a template + input context.
///
/// Reference: `omenic infra/prompts` fragment composition and
/// `dsh system-prompt.ts:666` prep step.
/// Constraint: pure function of `(template, input)` — no I/O, no mutation.
/// Non-goal: no conditional fragment gating (host-level concern, see
/// `infra/prompts` doc note about `buildSystemPrompt`).
pub trait PromptRenderer {
    fn render(&self, input: &PromptInput) -> String;
}

/// Render a template string with `{{system}}`, `{{user}}`, `{{history}}`
/// placeholders. The simplest renderer: string substitution.
///
/// Reference: `omenic infra/prompts/src/lib.rs` fragment doc note
/// ("callers compose fragments at the role layer").
/// Constraint: placeholders are literal `{{key}}`; no other syntax.
/// Non-goal: no conditional branching, no loop constructs, no
/// frontmatter stripping.
pub fn render_with_template(template: &str, input: &PromptInput) -> String {
    // `Message` serializes to `{"role":...,"content":...}` — the same shape
    // `llm::Message` puts on the wire, so `{{history}}` is OpenAI JSON.
    let history = serde_json::to_string(&input.history).unwrap_or_else(|_| "[]".to_string());
    template
        .replace("{{system}}", &input.system)
        .replace("{{user}}", &input.user)
        .replace("{{history}}", &history)
}
