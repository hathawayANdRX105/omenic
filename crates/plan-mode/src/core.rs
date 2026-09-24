//! Public API for plan mode.

use protocol::{AbortSignal, ToolError, ToolResult, ToolSpec};
use serde_json::Value;
use tools_harness::Tool;

use crate::port::{DynPlanReviewPort, ReviewError, ReviewOutcome};
use crate::state::PlanModeRuntime;

/// Constant service name for plan mode.
pub const PLAN_MODE_SERVICE: &str = "harness.plan-mode";

/// Service providing plan mode functionality.
#[derive(Clone)]
pub struct PlanModeService {
    runtime: PlanModeRuntime,
    review_port: DynPlanReviewPort,
    section: String,
}

impl PlanModeService {
    /// Create a new plan mode service.
    #[must_use]
    pub fn new(runtime: PlanModeRuntime, review_port: DynPlanReviewPort, section: String) -> Self {
        Self {
            runtime,
            review_port,
            section,
        }
    }

    /// Get the plan policy section for prompt injection.
    ///
    /// Returns the configured section when plan mode is active, empty string otherwise.
    #[must_use]
    pub fn plan_policy_section(&self) -> String {
        self.runtime.plan_policy_section(&self.section)
    }

    /// Get the current plan mode runtime.
    #[must_use]
    pub fn runtime(&self) -> &PlanModeRuntime {
        &self.runtime
    }

    /// Get the review port for plan approval.
    #[must_use]
    pub fn review_port(&self) -> &DynPlanReviewPort {
        &self.review_port
    }
}

/// Exit plan mode tool.
pub struct ExitPlanModeTool {
    name: String,
    runtime: PlanModeRuntime,
    review_port: DynPlanReviewPort,
}

impl ExitPlanModeTool {
    /// Create a new exit plan mode tool.
    #[must_use]
    pub fn new(name: String, runtime: PlanModeRuntime, review_port: DynPlanReviewPort) -> Self {
        Self {
            name,
            runtime,
            review_port,
        }
    }
}

impl Tool for ExitPlanModeTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name.clone(),
            description: "Exit plan mode after user review of the complete plan".to_string(),
            params_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "plan": {
                        "type": "string",
                        "description": "The complete plan to review and approve"
                    }
                },
                "required": ["plan"]
            }),
        }
    }

    fn execute(&self, args: &Value, _abort_signal: &AbortSignal) -> Result<ToolResult, ToolError> {
        // Check if plan mode is active
        if !self
            .runtime
            .active()
            .map_err(|e| ToolError::Execute(e.to_string()))?
        {
            return Err(ToolError::Execute(
                "exit_plan_mode is only available in Plan Mode".to_string(),
            ));
        }

        // Get plan from args
        let plan = match args.get("plan") {
            Some(plan) => plan.as_str().unwrap_or(""),
            None => "",
        };

        // Validate plan (non-empty and starts with '#')
        if plan.is_empty() || !plan.trim_start().starts_with('#') {
            return Err(ToolError::Execute(
                "Plan must be non-empty and start with '#' to be valid".to_string(),
            ));
        }

        // Review the plan
        match self.review_port.review(plan) {
            Ok(ReviewOutcome::Approved) => {
                // Plan approved, proceed to exit
                let prepared_exit = self
                    .runtime
                    .prepare_approved_exit()
                    .map_err(|e| ToolError::Execute(e.to_string()))?;
                prepared_exit
                    .commit()
                    .map_err(|e| ToolError::Execute(e.to_string()))?;
                Ok(ToolResult {
                    output: "Plan approved. Exiting plan mode.".to_string(),
                    is_error: false,
                })
            }
            Ok(ReviewOutcome::Rejected { feedback }) => {
                Err(ToolError::Execute(format!("Plan rejected: {}", feedback)))
            }
            Ok(ReviewOutcome::Dismissed) => Err(ToolError::Execute(
                "Plan review dismissed. Taking over control.".to_string(),
            )),
            Err(ReviewError::Transport(msg)) => Err(ToolError::Execute(format!(
                "Review transport unavailable: {}",
                msg
            ))),
            Err(ReviewError::Cancelled) => {
                Err(ToolError::Execute("Plan review cancelled".to_string()))
            }
        }
    }
}
