//! Plan review port tests.

use plan_mode::{AutoDenyReview, DynPlanReviewPort, PlanReviewPort, ReviewError, ReviewOutcome};

#[test]
fn auto_deny_review_always_rejects() {
    let port = AutoDenyReview;
    let result = port.review("any plan").unwrap_err();
    assert!(matches!(result, ReviewError::Transport(_)));
    // Transport is a raw passthrough: Display is the message itself.
    assert_eq!(result.to_string(), "review transport unavailable");
}

#[test]
fn review_outcome_variants() {
    let approved = ReviewOutcome::Approved;
    assert!(matches!(approved, ReviewOutcome::Approved));

    let rejected = ReviewOutcome::Rejected {
        feedback: "needs more detail".to_string(),
    };
    assert!(matches!(rejected, ReviewOutcome::Rejected { .. }));
    // Verify feedback is preserved
    if let ReviewOutcome::Rejected { feedback } = rejected {
        assert_eq!(feedback, "needs more detail");
    }

    let dismissed = ReviewOutcome::Dismissed;
    assert!(matches!(dismissed, ReviewOutcome::Dismissed));
}

#[test]
fn review_error_transport() {
    let err = ReviewError::Transport("connection failed".to_string());
    assert_eq!(err.to_string(), "connection failed");
}

#[test]
fn review_error_cancelled() {
    let err = ReviewError::Cancelled;
    assert_eq!(err.to_string(), "review cancelled");
}

#[test]
fn dyn_plan_review_port_works() {
    struct TestPort;
    impl PlanReviewPort for TestPort {
        fn review(&self, _plan: &str) -> Result<ReviewOutcome, ReviewError> {
            Ok(ReviewOutcome::Approved)
        }
    }

    let port: DynPlanReviewPort = Arc::new(TestPort);
    let result = port.review("test").unwrap();
    assert!(matches!(result, ReviewOutcome::Approved));
}

#[test]
fn review_outcome_clone_debug() {
    let approved = ReviewOutcome::Approved;
    let cloned = approved.clone();
    assert!(matches!(cloned, ReviewOutcome::Approved));
    let debug = format!("{:?}", approved);
    assert!(debug.contains("Approved"));

    let rejected = ReviewOutcome::Rejected {
        feedback: "test".to_string(),
    };
    let debug = format!("{:?}", rejected);
    assert!(debug.contains("Rejected"));
    assert!(debug.contains("test"));

    let dismissed = ReviewOutcome::Dismissed;
    let debug = format!("{:?}", dismissed);
    assert!(debug.contains("Dismissed"));
}

use std::sync::Arc;
