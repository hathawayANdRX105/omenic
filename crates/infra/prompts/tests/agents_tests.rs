use prompts::agents::*;

#[test]
fn designer_has_named_frontmatter() {
    assert!(
        DESIGNER.starts_with("---\n"),
        "designer.md must start with YAML frontmatter"
    );
    assert!(
        DESIGNER.contains("\nname: designer\n") || DESIGNER.contains("\nname: \"designer\""),
        "designer.md frontmatter must declare name: designer"
    );
}

#[test]
fn frontmatter_is_template() {
    assert!(
        FRONTMATTER.contains("{{"),
        "frontmatter.md is a Mustache template"
    );
}

#[test]
fn init_has_named_frontmatter() {
    assert!(
        INIT.starts_with("---\n"),
        "init.md must start with YAML frontmatter"
    );
    assert!(
        INIT.contains("\nname: init\n") || INIT.contains("\nname: \"init\""),
        "init.md frontmatter must declare name: init"
    );
}

#[test]
fn librarian_has_named_frontmatter() {
    assert!(
        LIBRARIAN.starts_with("---\n"),
        "librarian.md must start with YAML frontmatter"
    );
    assert!(
        LIBRARIAN.contains("\nname: librarian\n") || LIBRARIAN.contains("\nname: \"librarian\""),
        "librarian.md frontmatter must declare name: librarian"
    );
}

#[test]
fn reviewer_has_named_frontmatter() {
    assert!(
        REVIEWER.starts_with("---\n"),
        "reviewer.md must start with YAML frontmatter"
    );
    assert!(
        REVIEWER.contains("\nname: reviewer\n") || REVIEWER.contains("\nname: \"reviewer\""),
        "reviewer.md frontmatter must declare name: reviewer"
    );
}

#[test]
fn scout_has_named_frontmatter() {
    assert!(
        SCOUT.starts_with("---\n"),
        "scout.md must start with YAML frontmatter"
    );
    assert!(
        SCOUT.contains("\nname: scout\n") || SCOUT.contains("\nname: \"scout\""),
        "scout.md frontmatter must declare name: scout"
    );
}
