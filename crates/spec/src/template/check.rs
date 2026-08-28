//! Spec validation: check filled documents against template rules.

use std::path::Path;

use super::parse::parse_heading;
use super::{Spec, SpecField};

#[derive(Debug, Clone, PartialEq)]
pub struct CheckFinding {
    pub rule: &'static str,
    pub fail: bool,
    pub message: String,
}

impl CheckFinding {
    fn ok(rule: &'static str, message: String) -> Self {
        CheckFinding {
            rule,
            fail: false,
            message,
        }
    }
    fn fail(rule: &'static str, message: String) -> Self {
        CheckFinding {
            rule,
            fail: true,
            message,
        }
    }
}

/// Validate a filled spec document against the table's rules.
///
/// Checks, per field: required heading present; required content non-empty
/// (checkbox fields need at least one `- [ ]` / `- [x]` line; Construction
/// plan needs >= 2); `forbid_heading` must not appear. Also flags unknown
/// `##` headings with empty bodies. Optional fields may be absent/empty.
pub fn check(spec: &Spec, doc: &str) -> Vec<CheckFinding> {
    let mut findings = Vec::new();

    // Split into (heading, body-lines) sections at `## ` level; blank lines
    // inside a body are kept, leading/trailing blank lines dropped, and HTML
    // comment lines (skeleton fill hints) are excluded from content.
    let mut sections: Vec<(String, Vec<&str>)> = Vec::new();
    for line in doc.lines() {
        if line.starts_with("## ") {
            let (heading, _) = parse_heading(line);
            sections.push((heading, Vec::new()));
        } else if let Some((_, body)) = sections.last_mut() {
            let t = line.trim();
            if t.starts_with("<!--") {
                continue;
            }
            if !t.is_empty() || !body.is_empty() {
                body.push(line);
            }
        }
    }

    // CRG Review heading carries a `<english-title>` placeholder in the
    // template but real titles in filled docs — match by prefix there.
    let heading_matches = |want: &str, s: &str| {
        s == want
            || (want.starts_with("Agent 🤖 - CRG Review:")
                && s.starts_with("Agent 🤖 - CRG Review:"))
    };
    let has_heading = |h: &str| sections.iter().any(|(s, _)| heading_matches(h, s));
    let heading_body = |h: &str| -> Option<&Vec<&str>> {
        sections
            .iter()
            .find(|(s, _)| heading_matches(h, s))
            .map(|(_, b)| b)
    };

    for f in &spec.fields {
        if !has_heading(&f.heading) {
            if f.required {
                findings.push(CheckFinding::fail(
                    "SPEC-01",
                    format!("required heading missing: ## {}", f.heading),
                ));
            } else {
                findings.push(CheckFinding::ok(
                    "SPEC-01",
                    format!("optional heading absent: {}", f.heading),
                ));
            }
            continue;
        }
        let body: &[&str] = heading_body(&f.heading)
            .map(|b| b.as_slice())
            .unwrap_or(&[]);
        let content: String = body.join("\n");
        if content.trim().is_empty() {
            if f.required {
                findings.push(CheckFinding::fail(
                    "SPEC-01",
                    format!("field empty: {}", f.heading),
                ));
            } else {
                findings.push(CheckFinding::ok(
                    "SPEC-01",
                    format!("optional field empty: {}", f.heading),
                ));
            }
            continue;
        }
        if f.checkbox && f.heading != "Checklist" {
            let n_boxes = body
                .iter()
                .filter(|l| l.trim_start().starts_with("- ["))
                .count();
            if n_boxes == 0 {
                findings.push(CheckFinding::fail(
                    "SPEC-01",
                    format!("checkbox field needs `- [ ]` lines: {}", f.heading),
                ));
                continue;
            }
            if f.heading == "Construction plan" && n_boxes < 2 {
                findings.push(CheckFinding::fail(
                    "SPEC-01",
                    format!("Construction plan needs at least 2 checkboxes, found {n_boxes}"),
                ));
                continue;
            }
            findings.push(CheckFinding::ok(
                "SPEC-01",
                format!("{}: {} checkbox(es)", f.heading, n_boxes),
            ));
        } else {
            findings.push(CheckFinding::ok(
                "SPEC-01",
                format!("{}: filled", f.heading),
            ));
        }
    }

    if let Some(forbid) = &spec.forbid_heading {
        if has_heading(forbid) {
            findings.push(CheckFinding::fail(
                "SPEC-02",
                format!("forbidden heading present: ## {forbid}"),
            ));
        } else {
            findings.push(CheckFinding::ok(
                "SPEC-02",
                format!("no forbidden heading `{forbid}`"),
            ));
        }
    }

    for (h, body) in &sections {
        if !spec.fields.iter().any(|f| f.heading == *h) {
            if body.iter().all(|l| l.trim().is_empty()) {
                findings.push(CheckFinding::fail(
                    "SPEC-03",
                    format!("empty extra heading: ## {h}"),
                ));
            } else {
                findings.push(CheckFinding::ok(
                    "SPEC-03",
                    format!("extra heading ok: ## {h}"),
                ));
            }
        }
    }

    findings
}

/// Read a file and run `check`; convenience for the CLI layer.
pub fn check_file(spec: &Spec, path: &Path) -> Result<Vec<CheckFinding>, String> {
    let doc = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(check(spec, &doc))
}
