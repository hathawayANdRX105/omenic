//! Tool definitions for plan mode.

use crate::core::ExitPlanModeTool;
use crate::port::DynPlanReviewPort;
use crate::state::PlanModeRuntime;

/// Create the exit_plan_mode tool.
#[must_use]
pub fn exit_plan_mode_tool(
    runtime: PlanModeRuntime,
    review_port: DynPlanReviewPort,
) -> ExitPlanModeTool {
    ExitPlanModeTool::new("exit_plan_mode".to_string(), runtime, review_port)
}
