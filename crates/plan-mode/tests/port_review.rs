//! Review port contract: the default auto-deny transport and outcome shapes.

use plan_mode::{AutoDenyReview, PlanReviewPort, ReviewError, ReviewOutcome};

#[test]
fn auto_deny_reports_transport_unavailable() {
    let port = AutoDenyReview;
    let err = port.review("# any plan").unwrap_err();
    assert_eq!(
        err,
        ReviewError::Transport("review transport unavailable".to_string())
    );
    assert_eq!(err.to_string(), "review transport unavailable");
}

#[test]
fn outcomes_are_distinguishable() {
    assert_eq!(ReviewOutcome::Approved, ReviewOutcome::Approved);
    assert_ne!(
        ReviewOutcome::Rejected {
            feedback: "a".into()
        },
        ReviewOutcome::Dismissed
    );
}
