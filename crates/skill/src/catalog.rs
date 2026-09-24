//! Catalog service: entries(), catalog_message(), catalog_digest(), load().
//! Matches batch contract and dsh tool-skill spec (ponytail: FNV hash, literal template).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use thiserror::Error;

use crate::discovery::{SkillCatalogEntry, SkillRuntime, SkillRuntimeError};
use crate::parse::SkillLoadError;

/// Service key.
pub const SKILL_SERVICE: &str = "harness.skill";

/// Catalog error.
#[derive(Debug, Error)]
pub enum SkillServiceError {
    #[error("skill unavailable")]
    Unavailable(#[from] SkillRuntimeError),
    #[error("skill load failed: {0}")]
    Load(#[from] SkillLoadError),
}

/// Skill catalog service.
#[derive(Clone)]
pub struct SkillService {
    runtime: SkillRuntime,
}

impl SkillService {
    pub fn new() -> Self {
        Self {
            runtime: SkillRuntime::new(),
        }
    }

    pub fn with_cwd(cwd: impl Into<std::path::PathBuf>) -> Self {
        Self {
            runtime: SkillRuntime::with_cwd(cwd),
        }
    }

    /// Sorted model-invocable entries (ponytail: no model_invocable=false).
    pub fn entries(&self) -> Result<Vec<SkillCatalogEntry>, SkillServiceError> {
        self.runtime.entries().map_err(Into::into)
    }

    /// Official catalog message per tool-skill README (literal template).
    pub fn catalog_message(&self) -> Result<String, SkillServiceError> {
        let entries = self.entries()?;
        if entries.is_empty() {
            return Ok(String::new());
        }

        let skills_list = entries
            .iter()
            .map(|e| format!("- `{}`: {}", e.name, e.description))
            .collect::<Vec<_>>()
            .join("\n");

        let msg = format!(
            r#"<system-reminder>
A skill is a reusable set of task-specific instructions. The following skills are available in this session:

<available_skills>
{}
</available_skills>

If the user names a skill, or the task clearly matches a skill's description, call the `skill` tool with the exact skill name before taking task actions. Load all applicable skills, then follow their full instructions. This catalog contains summaries only; do not infer or follow a skill's instructions until it has been loaded.
A user may also invoke a skill directly; its <skill_content> block then appears in this conversation. Follow it, and do not call the `skill` tool again for that skill.
</system-reminder>"#,
            skills_list
        );
        Ok(msg)
    }

    /// Stable digest over (name, description) list. Uses std DefaultHasher (ponytail: no extra deps).
    pub fn catalog_digest(&self) -> Result<String, SkillServiceError> {
        let entries = self.entries()?;
        let mut hasher = DefaultHasher::new();
        for entry in &entries {
            entry.name.hash(&mut hasher);
            entry.description.hash(&mut hasher);
        }
        let hash = hasher.finish();
        Ok(format!("{:016x}", hash))
    }

    /// Load rendered skill (delegates to runtime).
    pub fn load(&self, name: &str) -> Result<String, SkillServiceError> {
        self.runtime.load(name).map_err(Into::into)
    }
}
