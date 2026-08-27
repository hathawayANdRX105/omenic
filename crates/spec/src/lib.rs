//! Spec tables: markdown skeletons for GitHub artifacts.
//!
//! Four kinds: issue, epic, pr, review.

//! Four kinds, stored as editable markdown template files under
//! `<data_dir>/specs/` (`.oi/specs/` in the default `.oi` layout):
//!
//! - `issue` — 拆分描述（Goal/Background/Suspected areas），验收与编排进 PR
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
//! <!-- desc: 普通 issue：拆分描述，无 Done when（验收在 PR）-->
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
<!-- desc: 普通 issue（task/bug/chore）：拆分描述，无 Done when；验收与编排在 PR，审查 findings 回写本 issue 评论区 -->

# <issue 标题>

## Goal [req]
<!-- 这个 issue 要解决的目标（必填） -->

## Background [req]
<!-- 为什么现在做、之前的决定或链接（必填） -->

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
        let content =
            std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
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
