//! Waterfall LLM fallback: try a primary OpenAI-compatible provider, and
//! only when a whole call to it fails *cleanly* (terminal `Error`, no
//! delta/tool-call emitted to the consumer) drop to the next configured
//! provider. Per-provider retries (exponential backoff) still run inside
//! each attempt; the waterfall is the layer above them.
//!
//! Placement note: the [`LlmBackend`] trait lives in this crate (orbit),
//! and adaptor is orbit's dependency — so the waterfall *runtime* must
//! live here, not in `adaptor::fallback`. `adaptor::openai::stream_cb_with_policy`
//! supplies the per-provider call; this module sequences the providers.

use std::sync::atomic::AtomicBool;

use adaptor::{Context, Model, StopReason, StreamEvent, ToolDef, openai::RetryPolicy};

use crate::LlmBackend;

/// One OpenAI-compatible LLM provider endpoint.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct LlmProvider {
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

impl LlmProvider {
    fn to_model(&self) -> Model {
        Model {
            api_key: self.api_key.clone(),
            model: self.model.clone(),
            base_url: self.base_url.clone(),
            max_tokens: self.max_tokens,
        }
    }
}

/// Waterfall runtime over [`LlmProvider`]s: `primary` first, then
/// `fallbacks` in listed order. A provider only yields to the next one
/// when its whole call fails without leaking any content to the consumer
/// (no `TextDelta`/`ToolCall` emitted); a partially-emitted turn is never
/// replayed on another provider, and aborts (consumer intent) stop
/// immediately.
#[derive(Debug, Clone)]
pub struct WaterfallLlm {
    pub primary: LlmProvider,
    pub fallbacks: Vec<LlmProvider>,
    pub retry: RetryPolicy,
}

impl WaterfallLlm {
    pub fn new(primary: LlmProvider, fallbacks: Vec<LlmProvider>) -> Self {
        Self {
            primary,
            fallbacks,
            retry: RetryPolicy::default(),
        }
    }

    /// Single-provider runtime, no fallbacks — equivalent to the historic
    /// `HttpLlm` behaviour (default retry policy).
    pub fn solo(primary: LlmProvider) -> Self {
        Self::new(primary, Vec::new())
    }

    /// Every provider in waterfall order: primary first, then fallbacks.
    fn providers(&self) -> Vec<&LlmProvider> {
        let mut out = vec![&self.primary];
        out.extend(self.fallbacks.iter());
        out
    }

    /// One provider's call plus the waterfall decision on it.
    ///
    /// Emits `true` when the round is finished (success, a clean abort, or a
    /// failure that already leaked content — in which case the error was
    /// just emitted and must not be retried elsewhere) and `false` when the
    /// provider failed *before emitting anything* and the next provider
    /// should take over.
    fn attempt_provider(
        provider: &LlmProvider,
        context: &Context,
        tools: &[ToolDef],
        signal: &AtomicBool,
        policy: RetryPolicy,
        emit: &mut dyn FnMut(&StreamEvent),
    ) -> bool {
        // Observe every event the per-provider call hands out: record
        // whether content leaked, whether the round ended in a clean abort,
        // and whether it ended in an Error — while forwarding *every* event
        // to the consumer via `emit`, unmodified, in order.
        let mut leaked_content = false;
        let mut aborted = false;
        let mut errored = false;
        adaptor::openai::stream_cb_with_policy(
            &provider.to_model(),
            context,
            tools,
            signal,
            &mut |ev| {
                match ev {
                    StreamEvent::TextDelta(_) | StreamEvent::ToolCall(_) => {
                        leaked_content = true;
                    }
                    StreamEvent::Done { stop_reason } => {
                        aborted = matches!(stop_reason, StopReason::Aborted);
                    }
                    StreamEvent::Error(_) => errored = true,
                }
                emit(ev);
            },
            policy,
        );
        if (errored && leaked_content) || aborted {
            // Content leaked before the error: the failure was just emitted
            // and must not be replayed elsewhere. A clean abort is consumer
            // intent, not a failure. Both stop the waterfall here.
            return true;
        }
        if errored {
            // Error, no content leaked, round ended: the next provider may
            // take over — the caller decides (continue or final error).
            return false;
        }
        // Clean `Done`: success.
        true
    }

    /// Run the waterfall. After all providers fail, emit one terminal
    /// `Error` naming the last provider's index; after any provider
    /// succeeds the round stops.
    fn run(
        &self,
        context: &Context,
        tools: &[ToolDef],
        signal: &AtomicBool,
        emit: &mut dyn FnMut(&StreamEvent),
    ) {
        let providers = self.providers();
        let last = providers.len() - 1;
        for (i, provider) in providers.iter().enumerate() {
            if Self::attempt_provider(provider, context, tools, signal, self.retry, emit) {
                return;
            }
            if i < last {
                continue;
            }
            // Every provider failed before leaking anything.
            emit(&StreamEvent::Error(format!(
                "llm provider {i} ({}) failed: all providers exhausted",
                provider.model
            )));
            return;
        }
    }
}

impl LlmBackend for WaterfallLlm {
    fn stream_cb(
        &self,
        _model: &Model,
        context: &Context,
        tools: &[ToolDef],
        signal: &AtomicBool,
        emit: &mut dyn FnMut(&StreamEvent),
    ) {
        // The providers carry their own models; the loop's `model` argument
        // is the primary's, used only for host-side metadata.
        self.run(context, tools, signal, emit);
    }
}
