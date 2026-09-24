//! User question broker for plan review and interactive prompts.
//!
//! Provides a daemon-hosted [`QuestionBroker`] that queues questions from
//! internal components (e.g. the plan-mode review port) and lets external
//! clients answer them via the `user.answer` protocol command. Submissions
//! are broadcast to subscribers through an `on_submit` callback the host
//! wires to its event bus.
//!
//! Bounds align with `deepseek-harness-rs/src/user_question.rs`.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use plan_mode::{PlanReviewPort, ReviewError, ReviewOutcome};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::now_ms;

/// Maximum bytes for the question id (aligns with dh-rs user_question.rs).
pub const MAX_QUESTION_ID_BYTES: usize = 64;
/// Maximum bytes for the question summary/header text.
pub const MAX_QUESTION_SUMMARY_BYTES: usize = 512;
/// Maximum bytes for the question detail/body text.
pub const MAX_QUESTION_DETAIL_BYTES: usize = 16 * 1024;
/// Maximum bytes for an option label.
pub const MAX_OPTION_LABEL_BYTES: usize = 128;
/// Maximum bytes for an option description.
pub const MAX_OPTION_DESCRIPTION_BYTES: usize = 256;
/// Maximum bytes for a custom feedback answer.
pub const MAX_CUSTOM_ANSWER_BYTES: usize = 4 * 1024;
/// Minimum number of options per question.
pub const MIN_QUESTION_OPTIONS: usize = 2;
/// Maximum number of options per question.
pub const MAX_QUESTION_OPTIONS: usize = 4;

/// A single selectable option in a question.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct QuestionOption {
    /// Display label for the option.
    pub label: String,
    /// Optional description for the option.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl QuestionOption {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            description: None,
        }
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }
}

/// Intent of the question — determines how answers are interpreted.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QuestionIntent {
    /// Plan review: user must Approve, Reject with feedback, or Dismiss.
    PlanReview,
    /// Generic user prompt with custom options.
    Prompt,
}

/// A question submitted to the broker.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct QuestionItem {
    /// Unique question identifier.
    pub id: String,
    /// Human-readable summary (max 512 bytes).
    pub summary: String,
    /// Optional detailed body text (max 16KB).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Intent of this question.
    pub intent: QuestionIntent,
    /// Selectable options (2-4 items).
    pub options: Vec<QuestionOption>,
    /// Timestamp when the question was submitted (ms since epoch).
    pub created_at_ms: i64,
}

impl QuestionItem {
    /// Create a plan review question (Approve / Reject / Dismiss).
    pub fn plan_review(plan_summary: &str, plan_detail: Option<String>) -> Self {
        QuestionItem {
            id: Uuid::new_v4().to_string(),
            summary: truncate(plan_summary, MAX_QUESTION_SUMMARY_BYTES),
            detail: plan_detail.map(|d| truncate(&d, MAX_QUESTION_DETAIL_BYTES)),
            intent: QuestionIntent::PlanReview,
            options: vec![
                QuestionOption::new("Approve").with_description("Accept the plan and proceed"),
                QuestionOption::new("Reject").with_description("Reject with feedback"),
                QuestionOption::new("Dismiss").with_description("Take over manually"),
            ],
            created_at_ms: now_ms(),
        }
    }

    /// Create a generic prompt question.
    pub fn prompt(
        summary: &str,
        detail: Option<String>,
        options: Vec<QuestionOption>,
    ) -> Result<Self, QuestionValidationError> {
        if options.len() < MIN_QUESTION_OPTIONS || options.len() > MAX_QUESTION_OPTIONS {
            return Err(QuestionValidationError::InvalidOptionCount);
        }
        for opt in &options {
            if opt.label.as_bytes().len() > MAX_OPTION_LABEL_BYTES {
                return Err(QuestionValidationError::OptionLabelTooLong);
            }
            if opt
                .description
                .as_ref()
                .is_some_and(|desc| desc.as_bytes().len() > MAX_OPTION_DESCRIPTION_BYTES)
            {
                return Err(QuestionValidationError::OptionDescriptionTooLong);
            }
        }
        Ok(QuestionItem {
            id: Uuid::new_v4().to_string(),
            summary: truncate(summary, MAX_QUESTION_SUMMARY_BYTES),
            detail: detail.map(|d| truncate(&d, MAX_QUESTION_DETAIL_BYTES)),
            intent: QuestionIntent::Prompt,
            options,
            created_at_ms: now_ms(),
        })
    }
}

/// Answer submitted by the user for a pending question.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QuestionAnswer {
    /// Selected option by index.
    Select { index: usize },
    /// Custom feedback text (for Reject with feedback).
    Custom { text: String },
}

/// Errors from question construction and validation.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum QuestionValidationError {
    #[error("question summary exceeds {0} bytes", MAX_QUESTION_SUMMARY_BYTES)]
    SummaryTooLong,
    #[error("question detail exceeds {0} bytes", MAX_QUESTION_DETAIL_BYTES)]
    DetailTooLong,
    #[error("option count must be between {MIN_QUESTION_OPTIONS} and {MAX_QUESTION_OPTIONS}")]
    InvalidOptionCount,
    #[error("option label exceeds {MAX_OPTION_LABEL_BYTES} bytes")]
    OptionLabelTooLong,
    #[error("option description exceeds {MAX_OPTION_DESCRIPTION_BYTES} bytes")]
    OptionDescriptionTooLong,
}

/// Errors returned when answering a question.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AnswerError {
    #[error("unknown question id: {0}")]
    UnknownQuestion(String),
    #[error("question already answered: {0}")]
    AlreadyAnswered(String),
    #[error("question timed out without an answer: {0}")]
    TimedOut(String),
    #[error("option index out of bounds")]
    IndexOutOfBounds,
    #[error("custom answer too long")]
    CustomAnswerTooLong,
    #[error("custom answer cannot be empty")]
    EmptyCustomAnswer,
}

impl From<AnswerError> for crate::protocol::ResponseError {
    fn from(e: AnswerError) -> Self {
        let code = match &e {
            AnswerError::UnknownQuestion(_) => "question_not_found",
            AnswerError::AlreadyAnswered(_) => "question_already_answered",
            AnswerError::TimedOut(_) => "question_timeout",
            AnswerError::IndexOutOfBounds => "invalid_option_index",
            AnswerError::CustomAnswerTooLong => "answer_too_long",
            AnswerError::EmptyCustomAnswer => "empty_answer",
        };
        crate::protocol::ResponseError::new(code, e.to_string())
    }
}

/// Handle returned when a question is submitted; blocks for the answer.
pub struct QuestionTicket {
    id: String,
    receiver: Receiver<QuestionAnswer>,
}

impl QuestionTicket {
    /// The question id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Block until answered or `timeout` elapses. A timeout is reported as
    /// [`AnswerError::TimedOut`] (distinct from an answer); the caller is
    /// responsible for removing the question via
    /// [`QuestionBroker::cancel`] — a dropped ticket alone would leak the
    /// pending entry.
    pub fn wait_timeout(&self, timeout: Duration) -> Result<QuestionAnswer, AnswerError> {
        self.receiver.recv_timeout(timeout).map_err(|e| match e {
            std::sync::mpsc::RecvTimeoutError::Timeout => AnswerError::TimedOut(self.id.clone()),
            std::sync::mpsc::RecvTimeoutError::Disconnected => {
                AnswerError::UnknownQuestion(self.id.clone())
            }
        })
    }
}

/// Internal pending question state.
struct PendingQuestion {
    item: QuestionItem,
    sender: Sender<QuestionAnswer>,
}

/// Callback fired (outside the broker lock) whenever a question is submitted.
type SubmitHook = Arc<dyn Fn(&QuestionItem) + Send + Sync>;

/// Configuration for the question broker.
#[derive(Clone, Debug)]
pub struct QuestionBrokerConfig {
    /// Timeout for plan review questions (default 10 minutes).
    pub plan_review_timeout: Duration,
}

impl Default for QuestionBrokerConfig {
    fn default() -> Self {
        Self {
            plan_review_timeout: Duration::from_secs(600), // 10 minutes
        }
    }
}

/// Thread-safe question broker.
#[derive(Clone)]
pub struct QuestionBroker {
    inner: Arc<Mutex<HashMap<String, PendingQuestion>>>,
    on_submit: Arc<Mutex<Option<SubmitHook>>>,
    config: QuestionBrokerConfig,
}

impl QuestionBroker {
    /// Create a new question broker with default configuration.
    pub fn new() -> Self {
        Self::with_config(QuestionBrokerConfig::default())
    }

    /// Create a new question broker with custom configuration.
    pub fn with_config(config: QuestionBrokerConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            on_submit: Arc::new(Mutex::new(None)),
            config,
        }
    }

    /// Install the submit hook (host wires it to the event bus broadcast).
    pub fn set_on_submit(&self, hook: impl Fn(&QuestionItem) + Send + Sync + 'static) {
        *self.on_submit.lock() = Some(Arc::new(hook));
    }

    /// Submit a question and get a ticket to wait for the answer.
    pub fn submit(&self, question: QuestionItem) -> QuestionTicket {
        let (sender, receiver) = channel();
        let id = question.id.clone();
        self.inner.lock().insert(
            id.clone(),
            PendingQuestion {
                item: question.clone(),
                sender,
            },
        );
        if let Some(hook) = self.on_submit.lock().as_ref() {
            hook(&question);
        }
        QuestionTicket { id, receiver }
    }

    /// Answer a pending question. Unknown ids (including already-answered
    /// ones, which are removed on first answer) are rejected.
    pub fn answer(&self, id: &str, answer: QuestionAnswer) -> Result<(), AnswerError> {
        let mut guard = self.inner.lock();
        let pending = match guard.get(id) {
            Some(p) => p,
            None => return Err(AnswerError::UnknownQuestion(id.to_string())),
        };

        match &answer {
            QuestionAnswer::Select { index } => {
                if *index >= pending.item.options.len() {
                    return Err(AnswerError::IndexOutOfBounds);
                }
            }
            QuestionAnswer::Custom { text } => {
                if text.as_bytes().len() > MAX_CUSTOM_ANSWER_BYTES {
                    return Err(AnswerError::CustomAnswerTooLong);
                }
                if text.trim().is_empty() {
                    return Err(AnswerError::EmptyCustomAnswer);
                }
            }
        }

        let pending = guard.remove(id).unwrap();
        pending
            .sender
            .send(answer)
            .map_err(|_| AnswerError::AlreadyAnswered(id.to_string()))
    }

    /// Remove a pending question without answering it. Idempotent; used by
    /// the review port after a wait failure (timeout) so an abandoned
    /// question cannot linger in `pending` forever — an unanswerable entry
    /// would keep surfacing to `user.question.pending` and the web card.
    pub fn cancel(&self, id: &str) {
        self.inner.lock().remove(id);
    }

    /// All pending questions, oldest first (deterministic for clients).
    pub fn pending(&self) -> Vec<QuestionItem> {
        let mut items: Vec<QuestionItem> =
            self.inner.lock().values().map(|p| p.item.clone()).collect();
        items.sort_by(|a, b| {
            a.created_at_ms
                .cmp(&b.created_at_ms)
                .then_with(|| a.id.cmp(&b.id))
        });
        items
    }

    /// A [`PlanReviewPort`] backed by this broker.
    pub fn review_port(self: &Arc<Self>) -> Arc<dyn PlanReviewPort> {
        Arc::new(BrokerReviewPort {
            broker: Arc::clone(self),
        })
    }
}

impl Default for QuestionBroker {
    fn default() -> Self {
        Self::new()
    }
}

/// [`PlanReviewPort`] implementation backed by the [`QuestionBroker`].
struct BrokerReviewPort {
    broker: Arc<QuestionBroker>,
}

impl PlanReviewPort for BrokerReviewPort {
    fn review(&self, plan: &str) -> Result<ReviewOutcome, ReviewError> {
        let question = QuestionItem::plan_review(plan, None);
        let ticket = self.broker.submit(question.clone());
        let answer = ticket
            .wait_timeout(self.broker.config.plan_review_timeout)
            .map_err(|e| {
                // The wait failed: drop the question so it cannot linger as
                // an unanswerable pending entry (timeout, or the broker
                // side went away).
                self.broker.cancel(&question.id);
                match e {
                    AnswerError::TimedOut(_) => ReviewError::Cancelled,
                    other => ReviewError::Transport(other.to_string()),
                }
            })?;

        match answer {
            QuestionAnswer::Select { index: 0 } => Ok(ReviewOutcome::Approved),
            QuestionAnswer::Select { index: 1 } => Ok(ReviewOutcome::Rejected {
                feedback: "Plan rejected. Ask the user what to change, then replan.".to_string(),
            }),
            QuestionAnswer::Select { index: 2 } => Ok(ReviewOutcome::Dismissed),
            QuestionAnswer::Select { .. } => {
                Err(ReviewError::Transport("invalid option index".to_string()))
            }
            QuestionAnswer::Custom { text } => {
                if text.trim().is_empty() {
                    Err(ReviewError::Transport("empty feedback".to_string()))
                } else {
                    Ok(ReviewOutcome::Rejected { feedback: text })
                }
            }
        }
    }
}

/// Truncate to at most `max_bytes`, ending on a char boundary.
fn truncate(s: &str, max_bytes: usize) -> String {
    if s.as_bytes().len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end < s.len() && !s.is_char_boundary(end) {
        end += 1;
    }
    s[..end].to_string()
}
