//! Bounded skill discovery using std::fs. Replicates deepseek-harness-rs discovery
//! semantics without cap-std/tokio/CancellationToken (ponytail: global sync, no async).

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::parse::{
    MAX_SKILL_FILE_BYTES, MAX_SKILL_NAME_BYTES, MAX_SKILL_ROOT_ENTRIES, MAX_SKILLS, ParsedSkill,
    SkillLoadError, parse_skill_file,
};

#[derive(Debug, Clone, Error)]
pub enum SkillRuntimeError {
    #[error("too many skills or I/O failure")]
    Unavailable,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Skill catalog entry (model-visible only).
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct SkillCatalogEntry {
    pub name: String,
    pub description: String,
}

/// Skill runtime (synchronous, cwd-based).
#[derive(Clone, Debug)]
pub struct SkillRuntime {
    cwd: PathBuf,
}

impl SkillRuntime {
    pub fn new() -> Self {
        Self {
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }

    pub fn with_cwd(cwd: impl Into<PathBuf>) -> Self {
        Self { cwd: cwd.into() }
    }

    /// Discover skills from both roots (`.dsh/skills` wins, `.agents/skills` secondary).
    /// Returns sorted, deduplicated, model-invocable entries (ponytail: BTreeMap for sort).
    pub fn entries(&self) -> Result<Vec<SkillCatalogEntry>, SkillRuntimeError> {
        let roots = [
            self.cwd.join(".dsh").join("skills"),
            self.cwd.join(".agents").join("skills"),
        ];

        let mut seen = HashSet::new();
        let mut entries = BTreeMap::new(); // name -> entry for sort + dedup

        for root in &roots {
            if !root.exists() {
                continue;
            }
            let mut count = 0;
            for entry in fs::read_dir(root)? {
                let entry = entry?;
                count += 1;
                if count > MAX_SKILL_ROOT_ENTRIES {
                    return Err(SkillRuntimeError::Unavailable);
                }

                let path = entry.path();
                if !self.is_safe_path(&path, root)? {
                    continue; // skip symlinks, etc.
                }

                let name = if let Some(n) = self.skill_name_from_path(&path) {
                    n
                } else {
                    continue;
                };

                if seen.contains(&name) {
                    continue; // first root wins
                }

                let content = match self.read_bounded(&path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                if let Some(parsed) = parse_skill_file(&content) {
                    if parsed.model_invocable && !seen.contains(&parsed.name) {
                        seen.insert(parsed.name.clone());
                        entries.insert(
                            parsed.name.clone(),
                            SkillCatalogEntry {
                                name: parsed.name,
                                description: parsed.description,
                            },
                        );
                    }
                }
            }
        }

        if entries.len() > MAX_SKILLS {
            return Err(SkillRuntimeError::Unavailable);
        }

        Ok(entries.into_values().collect())
    }

    fn is_safe_path(&self, path: &Path, root: &Path) -> Result<bool, SkillRuntimeError> {
        // Reject symlinks (ponytail: use symlink_metadata + component walk; no cap-std)
        let meta = match fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(_) => return Ok(false),
        };
        if meta.file_type().is_symlink() {
            return Ok(false);
        }

        // Simple component check
        if let Ok(rel) = path.strip_prefix(root) {
            for comp in rel.components() {
                if let std::path::Component::Normal(name) = comp {
                    if name.to_string_lossy().starts_with('.') && name != "." && name != ".." {
                        return Ok(false); // hidden
                    }
                }
            }
        }
        Ok(true)
    }

    fn skill_name_from_path(&self, path: &Path) -> Option<String> {
        let file_name = path.file_name()?.to_string_lossy();
        if file_name.ends_with(".md") && !file_name.starts_with('.') {
            let name = file_name.trim_end_matches(".md");
            if crate::parse::is_skill_name(name) {
                return Some(name.to_string());
            }
        } else if path.is_dir() {
            let dir_name = file_name;
            if crate::parse::is_skill_name(&dir_name) {
                let skill_md = path.join("SKILL.md");
                if skill_md.exists() {
                    return Some(dir_name.to_string());
                }
            }
        }
        None
    }

    fn read_bounded(&self, path: &Path) -> Result<String, SkillRuntimeError> {
        let meta = fs::metadata(path)?;
        if meta.len() as usize > MAX_SKILL_FILE_BYTES {
            return Err(SkillRuntimeError::Unavailable);
        }
        let content = fs::read_to_string(path)?;
        if content.len() > MAX_SKILL_FILE_BYTES {
            return Err(SkillRuntimeError::Unavailable);
        }
        Ok(content)
    }

    /// Load full rendered skill content.
    pub fn load(&self, name: &str) -> Result<String, SkillLoadError> {
        if name.is_empty()
            || name.len() > MAX_SKILL_NAME_BYTES
            || !crate::parse::is_skill_name(name)
        {
            return Err(SkillLoadError::InvalidName);
        }

        let roots = [
            self.cwd.join(".dsh").join("skills"),
            self.cwd.join(".agents").join("skills"),
        ];

        for root in &roots {
            // Try directory form first
            let dir_path = root.join(name);
            if dir_path.is_dir() {
                let md_path = dir_path.join("SKILL.md");
                if md_path.is_file() {
                    if let Ok(content) = self.read_bounded(&md_path) {
                        if let Some(parsed) = parse_skill_file(&content) {
                            if parsed.name == name {
                                if !parsed.model_invocable {
                                    return Err(SkillLoadError::NotModelInvocable);
                                }
                                return Ok(self.render_skill(&parsed, &dir_path));
                            }
                        }
                    }
                }
            }

            // Try flat .md
            let flat_path = root.join(format!("{}.md", name));
            if flat_path.is_file() {
                if let Ok(content) = self.read_bounded(&flat_path) {
                    if let Some(parsed) = parse_skill_file(&content) {
                        if parsed.name == name {
                            if !parsed.model_invocable {
                                return Err(SkillLoadError::NotModelInvocable);
                            }
                            return Ok(self.render_skill(&parsed, root));
                        }
                    }
                }
            }
        }

        Err(SkillLoadError::Unknown)
    }

    fn render_skill(&self, parsed: &ParsedSkill, resource_base: &Path) -> String {
        let base = resource_base.display();
        format!(
            r#"<skill_content name="{}">
<skill_resources>
Base directory for this skill: {}
Resolve relative paths mentioned by this skill against the base directory before using them. Load referenced resources only as needed.
</skill_resources>

<skill_instructions>
{}
</skill_instructions>
</skill_content>"#,
            parsed.name, base, parsed.body
        )
    }
}
