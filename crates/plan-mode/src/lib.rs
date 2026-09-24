//! Plan mode: logged per-agent collaboration state, the `/plan` command,
//! the reviewed `exit_plan_mode` tool, and the `plan:policy` prompt section.
//!
//! Reference: dsh `packages/plan/plan-mode` (behavioral spec) and
//! `deepseek-harness-rs/src/plan_mode.rs` (Rust reference implementation).

pub mod config;
pub mod core;
pub mod port;
pub mod state;
pub mod tools;

pub use config::PlanModeConfig;
pub use core::{ExitPlanModeTool, PLAN_MODE_SERVICE, PlanModeService};
pub use port::{AutoDenyReview, DynPlanReviewPort, PlanReviewPort, ReviewError, ReviewOutcome};
pub use state::{
    MAX_PLAN_BYTES, MAX_PLAN_COMMAND_MESSAGE_BYTES, PlanModeChange, PlanModeCommand, PlanModeError,
    PlanModeRuntime, PreparedPlanExit, PreparedPlanModeMutation,
};
