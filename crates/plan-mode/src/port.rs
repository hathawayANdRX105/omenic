//! Plan review port for user approval of plan exit.

use std::sync::Arc;

use thiserror::Error;

/// Outcome of a plan review.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReviewOutcome {
    /// Plan approved; caller may proceed to exit.
    Approved,
    /// Plan rejected with feedback; model should continue planning.
    Rejected { feedback: String },
    /// Review dismissed by user taking over; model should wait for user message.
    Dismissed,
}

/// Errors from the review transport.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum ReviewError {
    /// The review transport is unavailable.
    #[error("{0}")]
    Transport(String),
    /// The review was cancelled.
    #[error("review cancelled")]
    Cancelled,
}

/// Port for reviewing a plan before exiting plan mode.
///
/// Implementations are provided by the host (daemon) and injected via plugin config.
/// The default implementation [`AutoDenyReview`] rejects all plans.
pub trait PlanReviewPort: Send + Sync {
    /// Review a plan.
    ///
    /// Returns `Approved` if the plan is accepted,
    /// `Rejected { feedback }` if rejected with feedback,
    /// `Dismissed` if the user dismissed the review.
    fn review(&self, plan: &str) -> Result<ReviewOutcome, ReviewError>;
}

/// Default review port that auto-rejects all plans.
///
/// Used when no host-provided review transport is configured.
#[derive(Clone, Debug, Default)]
pub struct AutoDenyReview;

impl PlanReviewPort for AutoDenyReview {
    fn review(&self, _plan: &str) -> Result<ReviewOutcome, ReviewError> {
        Err(ReviewError::Transport(
            "review transport unavailable".to_string(),
        ))
    }
}

/// Trait object type for the plan review port.
pub type DynPlanReviewPort = Arc<dyn PlanReviewPort>;
