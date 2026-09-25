//! spec — GitHub artifact specification tables.
//!
//! `template`: spec table generation, parsing, rendering, validation
//! (issue / epic / pr / review markdown skeletons consumed by the CLI).

pub mod template;

// Re-export core types for backward compat (cli uses `spec::Spec`).
pub use template::{Spec, SpecField};
