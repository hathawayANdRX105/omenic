//! Spec tables: markdown skeletons for GitHub artifacts + gate validation.
//!
//! Two layers: template generation (init/parse/render/check) and
//! GitHub artifact compliance (rules/tools/shared).

pub mod check;
pub mod init;
pub mod parse;
pub mod render;
pub mod rules;
pub mod shared;
pub mod tools;

use std::path::Path;

/// One fillable field of a spec table.
#[derive(Debug, Clone, PartialEq)]
pub struct SpecField {
    /// Markdown heading text (matched at `##` level).
    pub heading: String,
    /// Required to be present and non-empty.
    pub required: bool,
    /// Field content must contain checkbox lines (`- [ ]` / `- [x]`).
    pub checkbox: bool,
    /// Filling hint rendered under the heading in generated skeletons.
    pub hint: String,
}

/// A spec table loaded from a template file.
#[derive(Debug, Clone)]
pub struct Spec {
    pub name: String,
    pub description: String,
    pub fields: Vec<SpecField>,
    /// Heading that must NOT appear in the filled document.
    pub forbid_heading: Option<String>,
}
