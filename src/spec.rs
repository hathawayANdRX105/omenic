//! Spec tables (规范表): pre-designed markdown skeletons for GitHub artifacts.
//!
//! Four kinds, aligned with `.github/ISSUE_TEMPLATE/` and gate rules
//! (`.githooks/spec/github_*.yaml`):
//!
//! - `issue` — must have **Done when** (checkbox acceptance)
//! - `epic`  — must have **Implement order**, must NOT contain **Done when**
//! - `pr`    — must have **Construction plan** (>= 2 checkboxes)
//! - `review`— CRG + ocr review format (`## Agent 🤖 - CRG Review: <title>`)
//!
//! Flow: `oi spec new <type>` generates the blank table → agent fills it →
//! `oi spec check <file>` validates headings / required fields / checkboxes.

use std::path::Path;

/// One fillable field of a spec table.
pub struct SpecField {
    /// Markdown heading text (matched at `##` level).
    pub heading: &'static str,
    /// Required to be present and non-empty.
    pub required: bool,
    /// Field content must contain checkbox lines (`- [ ]` / `- [x]`).
    pub checkbox: bool,
    /// Filling hint rendered under the heading in generated skeletons.
    pub hint: &'static str,
}

/// A named spec table.
pub struct Spec {
    pub name: &'static str,
    pub description: &'static str,
    /// First line of the generated skeleton (title placeholder).
    pub title_hint: &'static str,
    pub fields: &'static [SpecField],
    /// Heading that must NOT appear in the filled document.
    pub forbid_heading: Option<&'static str>,
}

pub const SPECS: &[Spec] = &[
    Spec {
        name: "issue",
        description: "普通 issue（task/bug/chore）：必须有 Done when",
        title_hint: "# <issue 标题>",
        fields: &[
            SpecField {
                heading: "Goal",
                required: true,
                checkbox: false,
                hint: "这个 issue 要解决的目标（必填）",
            },
            SpecField {
                heading: "Background",
                required: true,
                checkbox: false,
                hint: "为什么现在做、之前的决定或链接（必填）",
            },
            SpecField {
                heading: "Done when",
                required: true,
                checkbox: true,
                hint: "可观察的验收条件（必填）",
            },
            SpecField {
                heading: "Suspected areas",
                required: true,
                checkbox: false,
                hint: "改动范围：文件/package/符号/workflow/文档（必填）",
            },
            SpecField {
                heading: "Out of scope",
                required: false,
                checkbox: false,
                hint: "明确不应顺带纳入的工作（可选）",
            },
            SpecField {
                heading: "How to observe success",
                required: false,
                checkbox: false,
                hint: "命令/页面状态/CI job/指标或前后对比（可选）",
            },
            SpecField {
                heading: "Additional context",
                required: false,
                checkbox: false,
                hint: "无法放入上述字段的链接或脱敏说明（可选）",
            },
        ],
        forbid_heading: None,
    },
    Spec {
        name: "epic",
        description: "epic issue：必须有 Implement order，不能有 Done when",
        title_hint: "# <epic 标题>",
        fields: &[
            SpecField {
                heading: "Description",
                required: true,
                checkbox: false,
                hint: "里程碑目标：完成后应该存在什么能力（必填）",
            },
            SpecField {
                heading: "Problem / use case",
                required: true,
                checkbox: false,
                hint: "谁在当前流程中受阻、为什么拆这个里程碑（必填）",
            },
            SpecField {
                heading: "Implement order",
                required: true,
                checkbox: true,
                hint: "按顺序列出的实施步骤（必填；epic 用 Implement order，不用 Done when）",
            },
            SpecField {
                heading: "Scope",
                required: true,
                checkbox: false,
                hint: "单 PR 还是多 PR（必填）",
            },
            SpecField {
                heading: "Non-goals",
                required: false,
                checkbox: false,
                hint: "不应顺带纳入的 API/协议/部署/架构改动（可选）",
            },
            SpecField {
                heading: "Proposed approach",
                required: false,
                checkbox: false,
                hint: "高层方案（可选）",
            },
            SpecField {
                heading: "Alternatives considered",
                required: false,
                checkbox: false,
                hint: "被拒绝的设计或当前 workaround（可选）",
            },
            SpecField {
                heading: "Area",
                required: false,
                checkbox: false,
                hint: "负责该变化的 package/workflow/provider/区域（可选）",
            },
            SpecField {
                heading: "Additional context",
                required: false,
                checkbox: false,
                hint: "链接/先例/脱敏说明（可选）",
            },
        ],
        forbid_heading: Some("Done when"),
    },
    Spec {
        name: "pr",
        description: "PR：必须有 Construction plan（≥2 checkbox）",
        title_hint: "# <PR 标题>",
        fields: &[
            SpecField {
                heading: "What",
                required: true,
                checkbox: false,
                hint: "合并后会发生什么变化（必填）",
            },
            SpecField {
                heading: "Why",
                required: true,
                checkbox: false,
                hint: "为什么做、根因/背景/设计决策（必填）",
            },
            SpecField {
                heading: "Issue",
                required: true,
                checkbox: false,
                hint: "主 Issue：Fixes #N 或说明无关联（必填）",
            },
            SpecField {
                heading: "Construction plan",
                required: true,
                checkbox: true,
                hint: "最小实现步骤（必填，≥2 个 checkbox）",
            },
            SpecField {
                heading: "Delivery record",
                required: true,
                checkbox: false,
                hint: "Delivered / Verification / Follow-up（必填）",
            },
            SpecField {
                heading: "How to test",
                required: true,
                checkbox: false,
                hint: "评审者可复现的命令或步骤（必填）",
            },
            SpecField {
                heading: "Checklist",
                required: true,
                checkbox: true,
                hint: "提交前自检（必填）",
            },
        ],
        forbid_heading: None,
    },
    Spec {
        name: "review",
        description: "review：CRG + ocr 双审查格式",
        title_hint: "## Agent 🤖 - CRG Review: <english-title>",
        fields: &[
            SpecField {
                heading: "Agent 🤖 - CRG Review:",
                required: true,
                checkbox: false,
                hint: "CRG 审查标题（英文），发现按文件/严重度列出",
            },
            SpecField {
                heading: "ocr findings",
                required: true,
                checkbox: false,
                hint: "ocr AI 审查发现；无发现时写「无审查发现」",
            },
            SpecField {
                heading: "Conclusion",
                required: true,
                checkbox: false,
                hint: "结论：无阻塞项 / 需修复项清单",
            },
        ],
        forbid_heading: None,
    },
];

/// Look up a spec by name.
pub fn find(name: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|s| s.name == name)
}

/// Render the blank skeleton for a spec table.
pub fn render_skeleton(spec: &Spec, title: &str) -> String {
    let mut out = String::new();
    if spec.name == "review" {
        out.push_str(&format!("{}\n\n", spec.title_hint));
    } else {
        let t = if title.trim().is_empty() {
            spec.title_hint.to_string()
        } else {
            format!("# {title}")
        };
        out.push_str(&format!("{t}\n\n"));
    }
    for f in spec.fields {
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
    let mut sections: Vec<(&str, Vec<&str>)> = Vec::new();
    for line in doc.lines() {
        if line.starts_with("## ") {
            sections.push((line[3..].trim(), Vec::new()));
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

    let has_heading = |h: &str| sections.iter().any(|(s, _)| *s == h);
    let heading_body =
        |h: &str| -> Option<&Vec<&str>> { sections.iter().find(|(s, _)| *s == h).map(|(_, b)| b) };

    // Required fields: present + non-empty (+ checkbox lines for checkbox fields).
    // Optional fields: absent or empty is fine (reported as ok).
    for f in spec.fields {
        if !has_heading(f.heading) {
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
        let body: &[&str] = heading_body(f.heading).map(|b| b.as_slice()).unwrap_or(&[]);
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

    // Forbidden heading.
    if let Some(forbid) = spec.forbid_heading {
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

    // Unknown `##` headings with empty bodies.
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

    fn filled_issue() -> String {
        let mut doc = render_skeleton(find("issue").unwrap(), "test issue");
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
    fn skeleton_has_required_headings() {
        for spec in SPECS {
            let doc = render_skeleton(spec, "");
            for f in spec.fields {
                assert!(
                    doc.contains(&format!("## {}", f.heading)),
                    "{} missing heading {}",
                    spec.name,
                    f.heading
                );
            }
        }
    }

    #[test]
    fn empty_skeleton_fails_required_fields() {
        let spec = find("issue").unwrap();
        let doc = render_skeleton(spec, "x");
        let findings = check(spec, &doc);
        let fails = findings.iter().filter(|f| f.fail).count();
        // Goal / Background / Suspected areas are non-checkbox required
        // fields whose skeleton body is comments-only → empty → fail.
        assert!(
            fails >= 3,
            "expected >=3 fails, got {fails}: {:?}",
            findings
        );
    }

    #[test]
    fn filled_issue_passes() {
        let spec = find("issue").unwrap();
        let doc = filled_issue();
        let findings = check(spec, &doc);
        let fails: Vec<_> = findings.iter().filter(|f| f.fail).collect();
        assert!(fails.is_empty(), "unexpected fails: {:?}", fails);
    }

    #[test]
    fn epic_forbids_done_when() {
        let spec = find("epic").unwrap();
        let mut doc = render_skeleton(spec, "epic x");
        for (h, body) in [
            ("Description", "desc"),
            ("Problem / use case", "prob"),
            ("Implement order", "- [ ] a\n- [ ] b\n- [ ] c"),
            ("Scope", "multi"),
        ] {
            doc = doc.replace(&format!("## {h}\n"), &format!("## {h}\n{body}\n"));
        }
        let findings = check(spec, &doc);
        assert!(
            findings.iter().all(|f| !f.fail),
            "epic base should pass: {:?}",
            findings
        );

        // Adding Done when must fail.
        let bad = format!("{doc}\n## Done when\n- [ ] x\n");
        let findings = check(spec, &bad);
        assert!(
            findings
                .iter()
                .any(|f| f.fail && f.message.contains("Done when")),
            "epic must reject Done when: {:?}",
            findings
        );
    }

    #[test]
    fn pr_needs_two_checkboxes_in_construction_plan() {
        let spec = find("pr").unwrap();
        // Drop the skeleton's default single checkbox under Construction plan.
        let mut doc = render_skeleton(spec, "pr x")
            .replace(
                "## Construction plan\n<!-- 最小实现步骤（必填，≥2 个 checkbox） -->\n- [ ]\n",
                "## Construction plan\n",
            )
            .replace(
                "## Checklist\n<!-- 提交前自检（必填） -->\n- [ ]\n",
                "## Checklist\n",
            );
        for (h, body) in [
            ("What", "what text"),
            ("Why", "why text"),
            ("Issue", "Fixes #1"),
            ("Construction plan", "- [ ] only one\n"),
            (
                "Delivery record",
                "- Delivered: x\n- Verification: y\n- Follow-up: none",
            ),
            ("How to test", "cargo test"),
            ("Checklist", "- [x] a\n- [x] b\n- [x] c"),
        ] {
            doc = doc.replace(&format!("## {h}\n"), &format!("## {h}\n{body}\n"));
        }
        let findings = check(spec, &doc);
        assert!(
            findings
                .iter()
                .any(|f| f.fail && f.message.contains("at least 2")),
            "1-checkbox plan must fail: {:?}",
            findings
        );

        let doc2 = doc.replace("- [ ] only one\n", "- [ ] a\n- [ ] b\n");
        let findings = check(spec, &doc2);
        assert!(
            findings.iter().all(|f| !f.fail),
            "2-checkbox plan should pass: {:?}",
            findings
        );
    }

    #[test]
    fn empty_extra_heading_fails() {
        let spec = find("issue").unwrap();
        let doc = format!("{}\n## EmptySection\n", filled_issue());
        let findings = check(spec, &doc);
        assert!(
            findings
                .iter()
                .any(|f| f.fail && f.message.contains("EmptySection")),
            "{:?}",
            findings
        );
    }
}
