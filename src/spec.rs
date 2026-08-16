//! Spec tables (规范表): markdown skeletons for GitHub artifacts.
//!
//! Four kinds, stored as editable markdown template files under
//! `<data_dir>/specs/` (`.oi/specs/` in the default `.oi` layout):
//!
//! - `issue` — must have **Done when** (checkbox acceptance)
//! - `epic`  — must have **Implement order**, must NOT contain **Done when**
//! - `pr`    — must have **Construction plan** (>= 2 checkboxes)
//! - `review`— CRG + ocr review format
//!
//! Flow: `oi spec new <type>` generates the blank table → agent fills it →
//! `oi spec check <file>` validates headings / required fields / checkboxes.
//!
//! Template file format (self-describing):
//!
//! ```markdown
//! <!-- spec: issue -->
//! <!-- desc: 普通 issue：必须有 Done when -->
//! # <issue 标题>
//! ## Goal [req]
//! <!-- 提示文字 -->
//! ## Done when [req] [checkbox]
//! - [ ]
//! ```
//!
//! Heading flags: `[req]` / `[opt]` (default opt), `[checkbox]`.
//! `<!-- forbid: X -->` marks a heading that must not appear in the filled doc.

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

/// Default template file contents, written by `oi init` and used as fallback
/// when the specs dir is missing. Editable by the user afterwards.
pub const DEFAULT_TEMPLATES: &[(&str, &str)] = &[
    (
        "issue",
        r#"<!-- spec: issue -->
<!-- desc: 普通 issue（task/bug/chore）：必须有 Done when -->

# <issue 标题>

## Goal [req]
<!-- 这个 issue 要解决的目标（必填） -->

## Background [req]
<!-- 为什么现在做、之前的决定或链接（必填） -->

## Done when [req] [checkbox]
<!-- 可观察的验收条件（必填） -->
- [ ]

## Suspected areas [req]
<!-- 改动范围：文件/package/符号/workflow/文档（必填） -->

## Out of scope [opt]
<!-- 明确不应顺带纳入的工作（可选） -->

## How to observe success [opt]
<!-- 命令/页面状态/CI job/指标或前后对比（可选） -->

## Additional context [opt]
<!-- 无法放入上述字段的链接或脱敏说明（可选） -->
"#,
    ),
    (
        "epic",
        r#"<!-- spec: epic -->
<!-- desc: epic issue：必须有 Implement order，不能有 Done when -->
<!-- forbid: Done when -->

# <epic 标题>

## Description [req]
<!-- 里程碑目标：完成后应该存在什么能力（必填） -->

## Problem / use case [req]
<!-- 谁在当前流程中受阻、为什么拆这个里程碑（必填） -->

## Implement order [req] [checkbox]
<!-- 按顺序列出的实施步骤（必填；epic 用 Implement order，不用 Done when） -->
- [ ]

## Scope [req]
<!-- 单 PR 还是多 PR（必填） -->

## Non-goals [opt]
<!-- 不应顺带纳入的 API/协议/部署/架构改动（可选） -->

## Proposed approach [opt]
<!-- 高层方案（可选） -->

## Alternatives considered [opt]
<!-- 被拒绝的设计或当前 workaround（可选） -->

## Area [opt]
<!-- 负责该变化的 package/workflow/provider/区域（可选） -->

## Additional context [opt]
<!-- 链接/先例/脱敏说明（可选） -->
"#,
    ),
    (
        "pr",
        r#"<!-- spec: pr -->
<!-- desc: PR：必须有 Construction plan（≥2 checkbox） -->

# <PR 标题>

## What [req]
<!-- 合并后会发生什么变化（必填） -->

## Why [req]
<!-- 为什么做、根因/背景/设计决策（必填） -->

## Issue [req]
<!-- 主 Issue：Fixes #N 或说明无关联（必填） -->

## Construction plan [req] [checkbox]
<!-- 最小实现步骤（必填，≥2 个 checkbox） -->
- [ ]

## Delivery record [req]
<!-- Delivered / Verification / Follow-up（必填） -->

## How to test [req]
<!-- 评审者可复现的命令或步骤（必填） -->

## Checklist [req] [checkbox]
<!-- 提交前自检（必填） -->
- [ ]
"#,
    ),
    (
        "review",
        r#"<!-- spec: review -->
<!-- desc: review：CRG + ocr 双审查格式 -->

## Agent 🤖 - CRG Review: <english-title> [req]
<!-- CRG 审查标题（英文），发现按文件/严重度列出 -->

## ocr findings [req]
<!-- ocr AI 审查发现；无发现时写「无审查发现」 -->

## Conclusion [req]
<!-- 结论：无阻塞项 / 需修复项清单 -->
"#,
    ),
];

/// Write the four default templates into `<dir>/specs/` (idempotent — never
/// overwrites an existing file so user edits survive re-init).
pub fn write_default_specs(dir: &Path) -> Result<(), String> {
    let specs_dir = dir.join("specs");
    std::fs::create_dir_all(&specs_dir)
        .map_err(|e| format!("create specs dir {}: {e}", specs_dir.display()))?;
    for (name, content) in DEFAULT_TEMPLATES {
        let path = specs_dir.join(format!("{name}.md"));
        if !path.exists() {
            std::fs::write(&path, content)
                .map_err(|e| format!("write spec template {}: {e}", path.display()))?;
        }
    }
    Ok(())
}

/// Parse a spec template document into a `Spec`.
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
fn parse_heading(line: &str) -> (String, Vec<String>) {
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
        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        match parse_spec(&content) {
            Ok(spec) => specs.push(spec),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    Ok(specs)
}

/// Render the blank skeleton from a loaded spec (title substituted).
pub fn render_skeleton(spec: &Spec, title: &str) -> String {
    let mut out = String::new();
    if spec.name == "review" {
        out.push_str("## Agent 🤖 - CRG Review: <english-title>\n\n");
    } else {
        let t = if title.trim().is_empty() {
            "# <标题>".to_string()
        } else {
            format!("# {title}")
        };
        out.push_str(&format!("{t}\n\n"));
    }
    for f in &spec.fields {
        out.push_str(&format!("## {}\n", f.heading));
        if f.checkbox {
            out.push_str(&format!("<!-- {} -->\n- [ ]\n", f.hint));
        } else {
            out.push_str(&format!("<!-- {} -->\n\n", f.hint));
        }
        out.push('\n');
    }
    out
}

/// One check result.
#[derive(Debug, PartialEq)]
pub struct CheckFinding {
    pub rule: &'static str,
    pub fail: bool,
    pub message: String,
}

impl CheckFinding {
    fn ok(rule: &'static str, message: String) -> Self {
        CheckFinding { rule, fail: false, message }
    }
    fn fail(rule: &'static str, message: String) -> Self {
        CheckFinding { rule, fail: true, message }
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
        let body: &[&str] = heading_body(&f.heading).map(|b| b.as_slice()).unwrap_or(&[]);
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
            findings.push(CheckFinding::ok("SPEC-01", format!("{}: filled", f.heading)));
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn issue_spec() -> Spec {
        parse_spec(
            DEFAULT_TEMPLATES
                .iter()
                .find(|(n, _)| *n == "issue")
                .unwrap()
                .1,
        )
        .unwrap()
    }

    fn filled_issue() -> String {
        let spec = issue_spec();
        let mut doc = render_skeleton(&spec, "test issue");
        for (h, body) in [
            ("Goal", "goal text"),
            ("Background", "bg text"),
            ("Done when", "- [x] one\n- [ ] two"),
            ("Suspected areas", "src/foo.rs"),
        ] {
            doc = doc.replace(&format!("## {h}\n"), &format!("## {h}\n{body}\n"));
        }
        doc
    }

    #[test]
    fn parse_all_default_templates() {
        for (name, content) in DEFAULT_TEMPLATES {
            let spec = parse_spec(content).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(spec.name, *name);
            assert!(!spec.fields.is_empty(), "{name}: no fields");
        }
    }

    #[test]
    fn issue_spec_has_done_when_checkbox() {
        let spec = issue_spec();
        let done = spec.fields.iter().find(|f| f.heading == "Done when").unwrap();
        assert!(done.required && done.checkbox);
    }

    #[test]
    fn epic_spec_forbids_done_when() {
        let epic = parse_spec(DEFAULT_TEMPLATES.iter().find(|(n, _)| *n == "epic").unwrap().1).unwrap();
        assert_eq!(epic.forbid_heading.as_deref(), Some("Done when"));
    }

    #[test]
    fn write_default_specs_is_idempotent_and_parseable() {
        let tmp = tempdir().unwrap();
        write_default_specs(tmp.path()).unwrap();
        write_default_specs(tmp.path()).unwrap(); // second run: no overwrite
        let specs = load_all_specs(tmp.path()).unwrap();
        assert_eq!(specs.len(), 4);
        let names: Vec<_> = specs.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["epic", "issue", "pr", "review"]);
    }

    #[test]
    fn empty_skeleton_fails_required_fields() {
        let spec = issue_spec();
        let doc = render_skeleton(&spec, "x");
        let fails = check(&spec, &doc).iter().filter(|f| f.fail).count();
        // Goal / Background / Suspected areas are non-checkbox required
        // fields whose skeleton body is comments-only → empty → fail.
        assert!(fails >= 3, "expected >=3 fails, got {fails}");
    }

    #[test]
    fn filled_issue_passes() {
        let spec = issue_spec();
        let doc = filled_issue();
        let findings = check(&spec, &doc);
        let fails: Vec<_> = findings.iter().filter(|f| f.fail).collect();
        assert!(fails.is_empty(), "unexpected fails: {:?}", fails);
    }

    #[test]
    fn epic_forbids_done_when() {
        let spec = parse_spec(DEFAULT_TEMPLATES.iter().find(|(n, _)| *n == "epic").unwrap().1).unwrap();
        let mut doc = render_skeleton(&spec, "epic x");
        for (h, body) in [
            ("Description", "desc"),
            ("Problem / use case", "prob"),
            ("Implement order", "- [ ] a\n- [ ] b\n- [ ] c"),
            ("Scope", "multi"),
        ] {
            doc = doc.replace(&format!("## {h}\n"), &format!("## {h}\n{body}\n"));
        }
        let findings = check(&spec, &doc);
        assert!(
            findings.iter().all(|f| !f.fail),
            "epic base should pass: {:?}",
            findings
        );

        let bad = format!("{doc}\n## Done when\n- [ ] x\n");
        let findings = check(&spec, &bad);
        assert!(
            findings.iter().any(|f| f.fail && f.message.contains("Done when")),
            "epic must reject Done when: {:?}",
            findings
        );
    }

    #[test]
    fn pr_needs_two_checkboxes_in_construction_plan() {
        let spec = parse_spec(DEFAULT_TEMPLATES.iter().find(|(n, _)| *n == "pr").unwrap().1).unwrap();
        // Drop the skeleton's default single checkbox under Construction plan,
        // plus the Checklist default so only the test body counts.
        let mut doc = render_skeleton(&spec, "pr x")
            .replace(
                "## Construction plan\n<!-- 最小实现步骤（必填，≥2 个 checkbox） -->\n- [ ]\n",
                "## Construction plan\n",
            )
            .replace("## Checklist\n<!-- 提交前自检（必填） -->\n- [ ]\n", "## Checklist\n");
        for (h, body) in [
            ("What", "what text"),
            ("Why", "why text"),
            ("Issue", "Fixes #1"),
            ("Construction plan", "- [ ] only one\n"),
            ("Delivery record", "- Delivered: x\n- Verification: y\n- Follow-up: none"),
            ("How to test", "cargo test"),
            ("Checklist", "- [x] a\n- [x] b\n- [x] c"),
        ] {
            doc = doc.replace(&format!("## {h}\n"), &format!("## {h}\n{body}\n"));
        }
        let findings = check(&spec, &doc);
        assert!(
            findings.iter().any(|f| f.fail && f.message.contains("at least 2")),
            "1-checkbox plan must fail: {:?}",
            findings
        );

        let doc2 = doc.replace("- [ ] only one\n", "- [ ] a\n- [ ] b\n");
        let findings = check(&spec, &doc2);
        assert!(
            findings.iter().all(|f| !f.fail),
            "2-checkbox plan should pass: {:?}",
            findings
        );
    }

    #[test]
    fn empty_extra_heading_fails() {
        let spec = issue_spec();
        let doc = format!("{}\n## EmptySection\n", filled_issue());
        let findings = check(&spec, &doc);
        assert!(
            findings.iter().any(|f| f.fail && f.message.contains("EmptySection")),
            "{:?}",
            findings
        );
    }

    #[test]
    fn review_crg_heading_matches_real_title() {
        let spec = parse_spec(DEFAULT_TEMPLATES.iter().find(|(n, _)| *n == "review").unwrap().1).unwrap();
        let doc = "\
## Agent 🤖 - CRG Review: oi mvp agent loop
8 files, 0 findings.

## ocr findings
无审查发现

## Conclusion
无阻塞项
";
        let findings = check(&spec, doc);
        let fails: Vec<_> = findings.iter().filter(|f| f.fail).collect();
        assert!(fails.is_empty(), "review should pass with real title: {:?}", fails);
    }
}
