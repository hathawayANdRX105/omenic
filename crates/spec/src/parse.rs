//! Spec template parsing and loading.

use std::path::Path;

use crate::{Spec, SpecField};

pub fn parse_spec(content: &str) -> Result<Spec, String> {
    let mut name = String::new();
    let mut description = String::new();
    let mut forbid_heading: Option<String> = None;
    let mut fields: Vec<SpecField> = Vec::new();

    for line in content.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("<!-- spec:") {
            name = rest.trim_end_matches("-->").trim().to_string();
        } else if let Some(rest) = t.strip_prefix("<!-- desc:") {
            description = rest.trim_end_matches("-->").trim().to_string();
        } else if let Some(rest) = t.strip_prefix("<!-- forbid:") {
            forbid_heading = Some(rest.trim_end_matches("-->").trim().to_string());
        } else if t.starts_with("## ") {
            let (heading, flags) = parse_heading(t);
            fields.push(SpecField {
                heading,
                required: flags.iter().any(|f| f == "req"),
                checkbox: flags.iter().any(|f| f == "checkbox"),
                hint: String::new(),
            });
        } else if t.starts_with("<!--")
            && !t.starts_with("<!-- spec")
            && !t.starts_with("<!-- desc")
            && !t.starts_with("<!-- forbid")
        {
            if let Some(f) = fields.last_mut() {
                let hint = t
                    .trim_start_matches("<!--")
                    .trim_end_matches("-->")
                    .trim()
                    .to_string();
                if !hint.is_empty() {
                    f.hint = hint;
                }
            }
        }
    }

    if name.is_empty() {
        return Err("spec template missing `<!-- spec: <name> -->` marker".to_string());
    }
    if fields.is_empty() {
        return Err(format!("spec `{name}` has no `## ` fields"));
    }
    Ok(Spec {
        name,
        description,
        fields,
        forbid_heading,
    })
}

/// Split a `## Heading [flag1] [flag2]` line into (heading, flags).
pub(crate) fn parse_heading(line: &str) -> (String, Vec<String>) {
    let rest = &line[3..];
    let mut heading = String::new();
    let mut flags = Vec::new();
    for tok in rest.split_whitespace() {
        if tok.starts_with('[') && tok.ends_with(']') {
            flags.push(tok[1..tok.len() - 1].to_string());
        } else if heading.is_empty() {
            heading.push_str(tok);
        } else {
            heading.push(' ');
            heading.push_str(tok);
        }
    }
    (heading, flags)
}

/// Load one spec by name from `<dir>/specs/<name>.md`.
pub fn load_spec(dir: &Path, name: &str) -> Result<Spec, String> {
    let path = dir.join("specs").join(format!("{name}.md"));
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("read spec template {}: {e}", path.display()))?;
    let spec = parse_spec(&content)?;
    if spec.name != name {
        return Err(format!(
            "template file {} declares `{name}` but marker says `{}`",
            path.display(),
            spec.name
        ));
    }
    Ok(spec)
}

/// Load all spec templates from `<dir>/specs/` (skips non-`.md` files).
pub fn load_all_specs(dir: &Path) -> Result<Vec<Spec>, String> {
    let specs_dir = dir.join("specs");
    let mut specs = Vec::new();
    if !specs_dir.is_dir() {
        return Ok(specs);
    }
    let mut entries: Vec<_> = std::fs::read_dir(&specs_dir)
        .map_err(|e| format!("read specs dir {}: {e}", specs_dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "md").unwrap_or(false))
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let content =
            std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        match parse_spec(&content) {
            Ok(spec) => specs.push(spec),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    Ok(specs)
}
