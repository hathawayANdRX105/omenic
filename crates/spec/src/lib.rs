//! spec — GitHub artifact specification + compliance validation.
//!
//! Two layers:
//! - `template`: spec table generation, parsing, rendering, validation.
//! - `shared` / `rules` / `tools`: GitHub artifact compliance checks.

pub mod rules;
pub mod shared;
pub mod template;
pub mod tools;

// Re-export core types for backward compat (cli uses `spec::Spec`).
pub use template::{Spec, SpecField};
