//! Plan mode configuration.
//!
//! The plugin wrapper itself lives in `plugin::plugins` — this module is
//! only the configuration surface the host parses and hands over.

use crate::port::DynPlanReviewPort;

/// Configuration for the plan mode plugin.
#[derive(Clone, Default)]
pub struct PlanModeConfig {
    /// The plan policy section text (required, non-empty).
    pub section: Option<String>,
    /// Optional custom review port (if not provided, AutoDenyReview is used).
    pub review_port: Option<DynPlanReviewPort>,
}

impl PlanModeConfig {
    /// Validate the configuration.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(section) = &self.section {
            if section.trim().is_empty() {
                return Err("section must be non-empty".to_string());
            }
        } else {
            return Err("section is required".to_string());
        }
        Ok(())
    }
}
