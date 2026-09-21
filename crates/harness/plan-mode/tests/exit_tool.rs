//! exit_plan_mode tool gating and the review-port contract.

use std::sync::Arc;

use omenic_harness_core::{AbortSignal, Tool};
use omenic_harness_plan_mode::{
    AutoDenyReview, ExitPlanModeTool, PlanModeRuntime, PlanReviewPort, ReviewError, ReviewOutcome,
};
use serde_json::json;

struct ApproveAll;

impl PlanReviewPort for ApproveAll {
    fn review(&self, _plan: &str) -> Result<ReviewOutcome, ReviewError> {
        Ok(ReviewOutcome::Approved)
    }
}

fn active_tool(port: omenic_harness_plan_mode::DynPlanReviewPort) -> ExitPlanModeTool {
    let runtime = PlanModeRuntime::new(true);
    ExitPlanModeTool::new("exit_plan_mode".to_string(), runtime, port)
}

#[test]
fn spec_is_stable_and_advertises_plan_argument() {
    let tool = active_tool(Arc::new(AutoDenyReview));
    let spec = tool.spec();
    assert_eq!(spec.name, "exit_plan_mode");
    assert!(spec.description.contains("plan mode"));
    assert_eq!(spec.params_schema["required"][0], "plan");
}

#[test]
fn execute_rejected_when_plan_mode_inactive() {
    let tool = ExitPlanModeTool::new(
        "exit_plan_mode".to_string(),
        PlanModeRuntime::new(false),
        Arc::new(AutoDenyReview),
    );
    let err = tool
        .execute(&json!({ "plan": "# Ship it" }), &AbortSignal::new())
        .unwrap_err();
    match err {
        omenic_harness_core::ToolError::Execute(msg) => {
            assert!(msg.contains("only available in Plan Mode"), "{msg}");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn execute_rejects_plan_without_markdown_heading() {
    let tool = active_tool(Arc::new(ApproveAll));
    for bad in [json!({ "plan": "" }), json!({ "plan": "no heading here" })] {
        let err = tool.execute(&bad, &AbortSignal::new()).unwrap_err();
        assert!(matches!(err, omenic_harness_core::ToolError::Execute(_)));
    }
}

#[test]
fn auto_deny_port_rejects_valid_plan() {
    let tool = active_tool(Arc::new(AutoDenyReview));
    let err = tool
        .execute(&json!({ "plan": "# The plan" }), &AbortSignal::new())
        .unwrap_err();
    match err {
        omenic_harness_core::ToolError::Execute(msg) => {
            assert!(msg.contains("review transport unavailable"), "{msg}");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn approving_port_exits_and_parks_pending_boundary() {
    let runtime = PlanModeRuntime::new(true);
    let tool = ExitPlanModeTool::new(
        "exit_plan_mode".to_string(),
        runtime.clone(),
        Arc::new(ApproveAll),
    );
    let result = tool
        .execute(&json!({ "plan": "# The plan" }), &AbortSignal::new())
        .unwrap();
    assert!(!result.is_error);
    assert!(result.output.contains("Exiting plan mode"));

    // Approved exit parks a pending boundary; the next boundary commit lands it.
    let boundary = runtime.prepare_boundary().unwrap().unwrap();
    boundary.commit().unwrap();
    assert!(!runtime.active().unwrap());
}
