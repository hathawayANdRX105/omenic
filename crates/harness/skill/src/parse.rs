//! Skill file parsing: frontmatter + body extraction with bounds.
//!
//! Replicates `deepseek-harness-rs/src/skills.rs::parse_skill_file` semantics
//! (ponytail: hand-rolled line parser instead of serde_yaml — the format is a
//! tiny `key: value` subset, and strictness is the point: camelCase keys,
//! duplicate keys, indented lines, and non-boolean flags are rejected).

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

/// Max skills per workspace.
pub const MAX_SKILLS: usize = 64;

/// Max skill file size.
pub const MAX_SKILL_FILE_BYTES: usize = 256 * 1024;

/// Max entries scanned per skill root.
pub const MAX_SKILL_ROOT_ENTRIES: usize = 256;
/// Max skill name length in bytes.
pub const MAX_SKILL_NAME_BYTES: usize = 128;
/// Max catalog description length in chars.
pub const MAX_SKILL_DESCRIPTION_CHARS: usize = 500;

/// Errors from loading a skill by name.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum SkillLoadError {
    #[error("unknown or no longer available")]
    Unknown,
    #[error("skill is not available for model invocation")]
    NotModelInvocable,
    #[error("invalid skill name")]
    InvalidName,
    #[error("unavailable (too many entries or I/O error)")]
    Unavailable,
}

/// One parsed skill file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedSkill {
    pub name: String,
    pub description: String,
    pub model_invocable: bool,
    pub body: String,
}

/// Parse a skill file (frontmatter + body). `None` on any strictness violation.
pub fn parse_skill_file(raw: &str) -> Option<ParsedSkill> {
    let first_end = raw.find('\n')?;
    if raw[..first_end].trim_end_matches('\r') != "---" {
        return None;
    }
    let frontmatter_start = first_end + 1;
    let (frontmatter_end, body_start) = find_frontmatter_end(raw, frontmatter_start)?;
    let frontmatter = &raw[frontmatter_start..frontmatter_end];

    let mut fields = BTreeMap::<String, String>::new();
    let mut seen = BTreeSet::new();
    for raw_line in frontmatter.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if line.starts_with(char::is_whitespace) {
            return None;
        }
        let (key, value) = line.split_once(':')?;
        let key = key.trim();
        if key.is_empty() {
            return None;
        }
        if matches!(
            key,
            "disableModelInvocation" | "modelInvocable" | "userInvocable"
        ) {
            return None;
        }
        if !matches!(
            key,
            "name" | "description" | "whenToUse" | "disable-model-invocation" | "user-invocable"
        ) {
            continue;
        }
        if !seen.insert(key.to_owned()) {
            return None;
        }
        fields.insert(key.to_owned(), parse_scalar(value.trim())?);
    }

    let name = fields.remove("name")?;
    let description = fields.remove("description")?;
    if !is_skill_name(&name) || description.is_empty() {
        return None;
    }
    let disabled = match fields.remove("disable-model-invocation") {
        Some(value) => Some(parse_bool(&value)?),
        None => None,
    };
    Some(ParsedSkill {
        name,
        description: normalize_description(&description),
        model_invocable: disabled != Some(true),
        body: raw[body_start..].trim().to_owned(),
    })
}

/// Byte offset just past the closing `---` line, and where the body starts.
fn find_frontmatter_end(raw: &str, start: usize) -> Option<(usize, usize)> {
    let mut line_start = start;
    while line_start <= raw.len() {
        let next = raw[line_start..]
            .find('\n')
            .map(|offset| line_start + offset);
        let line_end = next.unwrap_or(raw.len());
        if raw[line_start..line_end].trim_end_matches('\r') == "---" {
            return Some((line_start, next.map_or(raw.len(), |index| index + 1)));
        }
        line_start = next? + 1;
    }
    None
}

fn parse_scalar(raw: &str) -> Option<String> {
    if raw.is_empty() || matches!(raw, "|" | ">" | "|-" | ">-" | "|+" | ">+") {
        return None;
    }
    if raw.starts_with('"') {
        return serde_json::from_str::<String>(raw).ok();
    }
    if raw.starts_with('\'') {
        if raw.len() < 2 || !raw.ends_with('\'') {
            return None;
        }
        return Some(raw[1..raw.len() - 1].replace("''", "'"));
    }
    if raw.starts_with(['[', '{', '&', '*', '!', '|', '>']) {
        return None;
    }
    Some(raw.to_owned())
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

/// Kebab-case name check: lowercase alnum + single hyphens, no edge hyphens.
pub fn is_skill_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_SKILL_NAME_BYTES {
        return false;
    }
    let bytes = name.as_bytes();
    if bytes.first() == Some(&b'-') || bytes.last() == Some(&b'-') {
        return false;
    }
    let mut previous_hyphen = false;
    for byte in bytes {
        match byte {
            b'a'..=b'z' | b'0'..=b'9' => previous_hyphen = false,
            b'-' if !previous_hyphen => previous_hyphen = true,
            _ => return false,
        }
    }
    true
}

/// Collapse whitespace, XML-escape, then cap at [`MAX_SKILL_DESCRIPTION_CHARS`].
pub fn normalize_description(value: &str) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let escaped = normalized
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let count = escaped.chars().count();
    if count > MAX_SKILL_DESCRIPTION_CHARS {
        let mut truncated: String = escaped
            .chars()
            .take(MAX_SKILL_DESCRIPTION_CHARS - 3)
            .collect();
        truncated.push_str("...");
        truncated
    } else {
        escaped
    }
}
