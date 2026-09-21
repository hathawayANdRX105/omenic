//! Discovery behavior: root priority, forms, bounds, symlink rejection.

use std::fs;

use omenic_harness_skill::discovery::{SkillRuntime, SkillRuntimeError};
use omenic_harness_skill::parse::SkillLoadError;
use tempfile::TempDir;

fn runtime_in(tmp: &TempDir) -> SkillRuntime {
    SkillRuntime::with_cwd(tmp.path())
}

#[test]
fn double_root_priority_and_first_win() {
    let tmp = TempDir::new().unwrap();
    let runtime = runtime_in(&tmp);
    let cwd = tmp.path();

    let agents = cwd.join(".agents/skills");
    fs::create_dir_all(&agents).unwrap();
    fs::write(
        agents.join("demo-skill.md"),
        "---\nname: demo-skill\ndescription: Lower priority\n---\nLower body.",
    )
    .unwrap();

    let dsh = cwd.join(".dsh/skills");
    fs::create_dir_all(&dsh).unwrap();
    fs::write(
        dsh.join("demo-skill.md"),
        "---\nname: demo-skill\ndescription: Winner priority\n---\nWinner body.",
    )
    .unwrap();

    let entries = runtime.entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "demo-skill");
    assert_eq!(entries[0].description, "Winner priority");

    let loaded = runtime.load("demo-skill").unwrap();
    assert!(loaded.contains("Winner body."));
    assert!(loaded.contains("Base directory for this skill:"));
}

#[test]
fn directory_and_file_forms_supported() {
    let tmp = TempDir::new().unwrap();
    let runtime = runtime_in(&tmp);

    let dsh = tmp.path().join(".dsh/skills");
    fs::create_dir_all(&dsh).unwrap();

    let dir = dsh.join("dir-skill");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("SKILL.md"),
        "---\nname: dir-skill\ndescription: Directory form\n---\nDir body.",
    )
    .unwrap();

    fs::write(
        dsh.join("file-skill.md"),
        "---\nname: file-skill\ndescription: File form\n---\nFile body.",
    )
    .unwrap();

    let entries = runtime.entries().unwrap();
    assert_eq!(entries.len(), 2);
    let names: Vec<_> = entries.iter().map(|e| &e.name).collect();
    assert_eq!(names, vec!["dir-skill", "file-skill"]);

    let dir_loaded = runtime.load("dir-skill").unwrap();
    assert!(dir_loaded.contains("Dir body."));
    let file_loaded = runtime.load("file-skill").unwrap();
    assert!(file_loaded.contains("File body."));
}

#[test]
fn boundaries_max_skills_unavailable_oversized_skipped_bad_frontmatter_skipped() {
    let tmp = TempDir::new().unwrap();
    let runtime = runtime_in(&tmp);

    let root = tmp.path().join(".dsh/skills");
    fs::create_dir_all(&root).unwrap();

    // More than MAX_SKILLS valid entries => Unavailable.
    for i in 0..65 {
        fs::write(
            root.join(format!("s-{}.md", i)),
            format!("---\nname: s-{}\ndescription: test\n---\nbody", i),
        )
        .unwrap();
    }
    assert!(matches!(
        runtime.entries(),
        Err(SkillRuntimeError::Unavailable)
    ));

    // Reset with a small valid set plus invalid neighbors.
    let tmp2 = TempDir::new().unwrap();
    let runtime2 = runtime_in(&tmp2);
    let root2 = tmp2.path().join(".dsh/skills");
    fs::create_dir_all(&root2).unwrap();
    fs::write(
        root2.join("good.md"),
        "---\nname: good\ndescription: Good\n---\nbody",
    )
    .unwrap();
    // Oversized file is skipped, not fatal.
    fs::write(root2.join("big.md"), vec![b'x'; 300_000]).unwrap();
    // Bad frontmatter is skipped, not fatal.
    fs::write(root2.join("bad.md"), "no-frontmatter").unwrap();

    let entries = runtime2.entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "good");
}

#[test]
fn disabled_skill_is_hidden_from_catalog_but_load_explains_why() {
    let tmp = TempDir::new().unwrap();
    let runtime = runtime_in(&tmp);

    let root = tmp.path().join(".dsh/skills");
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("quiet.md"),
        "---\nname: quiet\ndescription: Quiet\ndisable-model-invocation: true\n---\nbody",
    )
    .unwrap();

    assert!(runtime.entries().unwrap().is_empty());
    assert_eq!(
        runtime.load("quiet"),
        Err(SkillLoadError::NotModelInvocable)
    );
}

#[test]
fn unknown_and_invalid_names_are_rejected_on_load() {
    let tmp = TempDir::new().unwrap();
    let runtime = runtime_in(&tmp);

    assert_eq!(runtime.load("nope"), Err(SkillLoadError::Unknown));
    assert_eq!(runtime.load("Bad_Name"), Err(SkillLoadError::InvalidName));
    assert_eq!(runtime.load(""), Err(SkillLoadError::InvalidName));
}

#[test]
#[cfg(unix)]
fn symlinked_skill_files_are_rejected() {
    use std::os::unix::fs as unix_fs;

    let tmp = TempDir::new().unwrap();
    let runtime = runtime_in(&tmp);
    let root = tmp.path().join(".dsh/skills");
    fs::create_dir_all(&root).unwrap();

    let outside = TempDir::new().unwrap();
    fs::write(
        outside.path().join("linked.md"),
        "---\nname: linked\ndescription: Linked\n---\nOutside body.",
    )
    .unwrap();
    let link_path = root.join("linked.md");
    unix_fs::symlink(outside.path().join("linked.md"), &link_path).unwrap();

    assert_eq!(runtime.load("linked"), Err(SkillLoadError::Unknown));
    assert!(runtime.entries().unwrap().is_empty());
}
