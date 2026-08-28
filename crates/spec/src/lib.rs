//! spec — GitHub artifact specification + compliance validation.
//!
//! Two layers:
//! - `template`: spec table generation, parsing, rendering, validation.
//! - `gate`: GitHub artifact compliance (rules, tools, shared API helpers).

pub mod gate;
pub mod template;

// Re-export core types for backward compatibility (cli uses `spec::Spec`).
pub use template::{Spec, SpecField};
