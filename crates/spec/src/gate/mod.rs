//! Gate layer: GitHub artifact compliance checks.
//!
//! Three parts:
//! - `shared`: Severity/Finding types, gh_api client, YAML loader, run_external.
//! - `rules`: pure content checks for issues, PRs, reviews (driven by .githooks/spec/*.yaml).
//! - `tools`: validation runners (audit, cleanup, code, docs, merge, review, etc.).

pub mod rules;
pub mod shared;
pub mod tools;
