//! SkillTool behavior and the catalog service surface.

use std::fs;
use std::sync::Arc;

use omenic_harness_skill::{SkillService, SkillTool};
use omenic_harness_tools::Tool;
use serde_json::json;
use tempfile::TempDir;

fn workspace_with_skill(name: &str, frontmatter: &str, body: &str) -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dsh = tmp.path().join(".dsh/skills");
    fs::create_dir_all(&dsh).unwrap();
    // Front-matter needs its closing `---` (the parser requires it, same as
    // the reference).
    fs::write(
        dsh.join(format!("{name}.md")),
        format!("{frontmatter}\n---\n{body}"),
    )
    .unwrap();
    tmp
}

#[test]
fn tool_spec_names_the_skill_tool() {
    let tmp = TempDir::new().unwrap();
    let tool = SkillTool::new(Arc::new(SkillService::with_cwd(tmp.path())));
    let spec = tool.spec();
    assert_eq!(spec.name, "skill");
    assert_eq!(spec.params_schema["required"][0], "name");
}

#[test]
fn tool_rejects_invalid_and_unknown_names() {
    let tmp = TempDir::new().unwrap();
    let tool = SkillTool::new(Arc::new(SkillService::with_cwd(tmp.path())));

    let res = tool
        .execute(&json!({ "name": "" }), &Default::default())
        .unwrap();
    assert!(res.is_error);
    assert!(res.output.contains("invalid skill name"));

    let res = tool
        .execute(&json!({ "name": "Bad_Name" }), &Default::default())
        .unwrap();
    assert!(res.is_error);
    assert!(res.output.contains("invalid skill name"));

    let res = tool
        .execute(&json!({ "name": "nonexistent" }), &Default::default())
        .unwrap();
    assert!(res.is_error);
    assert!(res.output.contains("unknown or no longer available"));
}

#[test]
fn tool_renders_loaded_skill_content() {
    let tmp = workspace_with_skill(
        "test-skill",
        "---\nname: test-skill\ndescription: Test skill",
        "Do the test.",
    );
    let tool = SkillTool::new(Arc::new(SkillService::with_cwd(tmp.path())));

    let res = tool
        .execute(&json!({ "name": "test-skill" }), &Default::default())
        .unwrap();
    assert!(!res.is_error);
    assert!(res.output.contains("<skill_content name=\"test-skill\">"));
    assert!(res.output.contains("Do the test."));
    assert!(res.output.contains("Base directory for this skill:"));
}

#[test]
fn tool_reports_disabled_skill_distinctly() {
    let tmp = workspace_with_skill(
        "quiet",
        "---\nname: quiet\ndescription: Quiet\ndisable-model-invocation: true",
        "body",
    );
    let tool = SkillTool::new(Arc::new(SkillService::with_cwd(tmp.path())));

    let res = tool
        .execute(&json!({ "name": "quiet" }), &Default::default())
        .unwrap();
    assert!(res.is_error);
    assert!(res.output.contains("not available for model invocation"));
}

#[test]
fn catalog_message_uses_official_template() {
    let tmp = workspace_with_skill("demo", "---\nname: demo\ndescription: Demo skill", "Body.");
    let service = SkillService::with_cwd(tmp.path());

    let msg = service.catalog_message().unwrap();
    assert!(msg.contains("<system-reminder>"));
    assert!(msg.contains("<available_skills>"));
    assert!(msg.contains("- `demo`: Demo skill"));
    assert!(msg.contains("do not infer or follow a skill's instructions until it has been loaded"));
    assert!(msg.contains("do not call the `skill` tool again for that skill"));
}

#[test]
fn empty_workspace_has_empty_catalog_message() {
    let tmp = TempDir::new().unwrap();
    let service = SkillService::with_cwd(tmp.path());
    assert_eq!(service.catalog_message().unwrap(), "");
}

#[test]
fn catalog_digest_tracks_entry_changes() {
    let tmp = workspace_with_skill("demo", "---\nname: demo\ndescription: Demo skill", "Body.");
    let service = SkillService::with_cwd(tmp.path());

    let digest1 = service.catalog_digest().unwrap();
    assert_eq!(digest1.len(), 16);

    fs::write(
        tmp.path().join(".dsh/skills/demo.md"),
        "---\nname: demo\ndescription: Changed description\n---\nBody.",
    )
    .unwrap();

    let digest2 = service.catalog_digest().unwrap();
    assert_ne!(digest1, digest2);

    let digest3 = service.catalog_digest().unwrap();
    assert_eq!(digest2, digest3, "unchanged entries keep the digest stable");
}
