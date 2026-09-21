//! Project-local skills: bounded discovery over `.dsh/skills` and
//! `.agents/skills`, a digest-deduplicated catalog, and the `skill` tool.
//!
//! Reference: `deepseek-harness-rs/src/skills.rs` (Rust implementation) and
//! dsh `packages/skill/tool-skill` (catalog lifecycle spec).

pub mod catalog;
pub mod discovery;
pub mod parse;
pub mod plugin;
pub mod tool;

pub use catalog::{SKILL_SERVICE, SkillCatalogEntry, SkillService, SkillServiceError};
pub use discovery::{SkillRuntime, SkillRuntimeError};
pub use parse::{SkillLoadError, parse_skill_file};
pub use plugin::SkillPlugin;
pub use tool::SkillTool;
