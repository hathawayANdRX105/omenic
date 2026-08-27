use spec::check::CheckFinding;
use spec::check::check;
use spec::init::*;
use spec::parse::*;
use spec::render::*;
use spec::*;
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
fn issue_spec_has_no_done_when() {
    let spec = issue_spec();
    assert!(
        !spec.fields.iter().any(|f| f.heading == "Done when"),
        "issue spec must not require Done when (acceptance lives in the PR)"
    );
    assert!(spec.fields.iter().any(|f| f.heading == "Suspected areas"));
}

#[test]
fn epic_spec_forbids_done_when() {
    let epic = parse_spec(
        DEFAULT_TEMPLATES
            .iter()
            .find(|(n, _)| *n == "epic")
            .unwrap()
            .1,
    )
    .unwrap();
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
    let spec = parse_spec(
        DEFAULT_TEMPLATES
            .iter()
            .find(|(n, _)| *n == "epic")
            .unwrap()
            .1,
    )
    .unwrap();
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
        findings
            .iter()
            .any(|f| f.fail && f.message.contains("Done when")),
        "epic must reject Done when: {:?}",
        findings
    );
}

#[test]
fn pr_needs_two_checkboxes_in_construction_plan() {
    let spec = parse_spec(
        DEFAULT_TEMPLATES
            .iter()
            .find(|(n, _)| *n == "pr")
            .unwrap()
            .1,
    )
    .unwrap();
    // Drop the skeleton's default single checkbox under Construction plan,
    // plus the Checklist default so only the test body counts.
    let mut doc = render_skeleton(&spec, "pr x")
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
    let findings = check(&spec, &doc);
    assert!(
        findings
            .iter()
            .any(|f| f.fail && f.message.contains("at least 2")),
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
        findings
            .iter()
            .any(|f| f.fail && f.message.contains("EmptySection")),
        "{:?}",
        findings
    );
}

#[test]
fn review_crg_heading_matches_real_title() {
    let spec = parse_spec(
        DEFAULT_TEMPLATES
            .iter()
            .find(|(n, _)| *n == "review")
            .unwrap()
            .1,
    )
    .unwrap();
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
    assert!(
        fails.is_empty(),
        "review should pass with real title: {:?}",
        fails
    );
}
