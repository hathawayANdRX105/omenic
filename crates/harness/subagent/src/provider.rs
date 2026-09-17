use std::sync::Arc;

/// Which START-TIME features a provider supports.
///
/// Mirrors dsh `SubagentCapabilities`. Each flag corresponds to a field on
/// [`SubagentStartRequest`]; a request needing an unsupported capability is
/// rejected before `start` runs.
#[derive(Debug, Clone, Copy, Default)]
pub struct SubagentCapabilities {
    pub output_schema: bool,
    pub depth_limit: bool,
    pub tool_filter: bool,
    pub persona: bool,
}

/// What a caller asks for when starting a ONE-SHOT subagent.
///
/// For Phase 1 only `prompt` and `signal` are consumed by the fork backend.
/// The remaining fields are reserved for Phase 2 structured-output, depth,
/// tool-filter, and persona gating.
#[derive(Debug, Clone, Default)]
pub struct SubagentStartRequest {
    /// Content delivered as the child's user message.
    pub prompt: String,
    /// Cancellation signal from the spawning context.
    pub signal: Arc<std::sync::atomic::AtomicBool>,
    /// Whether the caller expects the child to inherit the parent's
    /// completed-turn prefix. Descriptive only for Phase 1 — the fork
    /// backend always starts the child fresh.
    pub inherits_parent_context: bool,
}

/// Terminal outcome of a subagent run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentResult {
    /// Child finished normally; `output` is the collected assistant text.
    Completed { output: String },
    /// Child or infrastructure failed; `error` is a human-readable summary.
    Failed { error: String },
    /// Run was cancelled via `SubagentRun::dispose`.
    Aborted,
}

/// A running subagent handle.
///
/// Usable from synchronous code: spawn a thread, send the terminal
/// [`SubagentResult`] through an `mpsc` channel, and block the caller on
/// `recv()`. Call [`SubagentRun::dispose`] to set the abort signal the worker
/// thread polls between turns.
pub struct SubagentRun {
    rx: std::sync::mpsc::Receiver<SubagentResult>,
    _join: std::thread::JoinHandle<()>,
}

impl SubagentRun {
    /// Construct a run from its receiver and worker handle.
    pub fn new(
        rx: std::sync::mpsc::Receiver<SubagentResult>,
        _join: std::thread::JoinHandle<()>,
    ) -> Self {
        Self { rx, _join }
    }

    /// Block until the worker thread sends its terminal [`SubagentResult`].
    ///
    /// If the channel is closed without a result (worker panicked before
    /// sending), returns [`SubagentResult::Failed`].
    pub fn result(&self) -> SubagentResult {
        self.rx.recv().unwrap_or(SubagentResult::Failed {
            error: "channel closed".into(),
        })
    }

    /// Signal the worker thread to abort.
    ///
    /// Idempotent. `signal` is the same [`std::sync::atomic::AtomicBool`]
    /// passed into [`SubagentStartRequest`]; the worker polls it at the top of
    /// each turn and after each `run_agent_streaming` call.
    pub fn dispose(&self, signal: &std::sync::atomic::AtomicBool) {
        signal.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// One registered transport for running child agents.
///
/// Providers are trusted same-process implementations. The service may call
/// one provider concurrently for distinct children.
pub trait SubagentProvider: Send + Sync {
    /// Unique registry name (e.g. `fork`, `spawn`, `acp`).
    fn name(&self) -> &str;

    /// The start-time features this provider supports.
    fn capabilities(&self) -> SubagentCapabilities;

    /// Whether the child sees the parent's completed-turn prefix.
    ///
    /// Descriptive only for Phase 1 — the fork backend ignores the seed.
    fn inherits_parent_context(&self) -> bool;

    /// Establish a ONE-SHOT child and return its handle after publication.
    fn start(&self, request: SubagentStartRequest) -> SubagentRun;
}
