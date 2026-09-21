//! Skill tool (wraps SkillService for ToolExecutor).

use std::sync::Arc;

use omenic_harness_core::{ToolError, ToolResult, ToolSpec};
use omenic_harness_tools::Tool;
use serde_json::Value;

use crate::catalog::{SkillService, SkillServiceError};
use crate::parse::SkillLoadError;

/// Skill tool (resolves service at runtime).
#[derive(Clone)]
pub struct SkillTool {
    service: Arc<SkillService>,
}

impl SkillTool {
    pub fn new(service: Arc<SkillService>) -> Self {
        Self { service }
    }
}

impl Tool for SkillTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "skill".to_string(),
            description: "Load a skill by exact name from the workspace catalog.".to_string(),
            params_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Exact kebab-case skill name from the available skills listing."
                    }
                },
                "required": ["name"]
            }),
        }
    }

    fn execute(
        &self,
        args: &Value,
        _abort: &omenic_harness_core::AbortSignal,
    ) -> Result<ToolResult, ToolError> {
        let name = match args.get("name").and_then(|v| v.as_str()) {
            Some(n) if !n.trim().is_empty() => n.trim(),
            _ => {
                return Ok(ToolResult {
                    output: "Error: invalid skill name".to_string(),
                    is_error: true,
                });
            }
        };

        match self.service.load(name) {
            Ok(content) => Ok(ToolResult {
                output: content,
                is_error: false,
            }),
            Err(SkillServiceError::Load(SkillLoadError::InvalidName)) => Ok(ToolResult {
                output: format!("Error: invalid skill name \"{}\"", name),
                is_error: true,
            }),
            Err(SkillServiceError::Load(SkillLoadError::Unknown)) => Ok(ToolResult {
                output: format!(
                    "Error: skill \"{}\" is unknown or no longer available",
                    name
                ),
                is_error: true,
            }),
            Err(SkillServiceError::Load(SkillLoadError::NotModelInvocable)) => Ok(ToolResult {
                output: format!(
                    "Error: skill \"{}\" is not available for model invocation",
                    name
                ),
                is_error: true,
            }),
            Err(e) => Ok(ToolResult {
                output: format!("Error: {}", e),
                is_error: true,
            }),
        }
    }
}
