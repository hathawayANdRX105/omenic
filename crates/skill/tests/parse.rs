//! Frontmatter strictness, bounds, and invocation flags (dh-rs semantics).

use skill::parse::{
    MAX_SKILL_DESCRIPTION_CHARS, is_skill_name, normalize_description, parse_skill_file,
};

#[test]
fn common_frontmatter_and_fail_closed_invocation_are_parsed() {
    let parsed = parse_skill_file(
        "---\nname: demo-skill\ndescription: 'Use  the demo.'\nuser-invocable: yes\n---\n\nFollow it.\n",
    )
    .unwrap();
    assert_eq!(parsed.name, "demo-skill");
    assert_eq!(parsed.description, "Use the demo.");
    assert!(parsed.model_invocable);
    assert_eq!(parsed.body.trim(), "Follow it.");

    // Non-boolean flag value fails closed.
    assert!(
        parse_skill_file(
            "---\nname: demo\ndescription: Demo\ndisable-model-invocation: maybe\n---\nbody"
        )
        .is_none()
    );

    // camelCase invocation keys are rejected outright.
    assert!(
        parse_skill_file("---\nname: demo\ndescription: Demo\nmodelInvocable: true\n---\nbody")
            .is_none()
    );
    assert!(
        parse_skill_file(
            "---\nname: demo\ndescription: Demo\ndisableModelInvocation: true\n---\nbody"
        )
        .is_none()
    );
}

#[test]
fn disable_model_invocation_flag_is_honored() {
    let parsed = parse_skill_file(
        "---\nname: demo\ndescription: Demo\ndisable-model-invocation: true\n---\nbody",
    )
    .unwrap();
    assert!(!parsed.model_invocable);

    let parsed = parse_skill_file(
        "---\nname: demo\ndescription: Demo\ndisable-model-invocation: no\n---\nbody",
    )
    .unwrap();
    assert!(parsed.model_invocable);
}

#[test]
fn malformed_frontmatter_is_rejected() {
    // Missing opening delimiter.
    assert!(parse_skill_file("name: demo\ndescription: d\n---\nbody").is_none());
    // Missing name / empty description.
    assert!(parse_skill_file("---\ndescription: d\n---\nbody").is_none());
    assert!(parse_skill_file("---\nname: demo\ndescription:\n---\nbody").is_none());
    // Indented line.
    assert!(parse_skill_file("---\nname: demo\n  description: d\n---\nbody").is_none());
    // Duplicate key.
    assert!(parse_skill_file("---\nname: demo\nname: other\ndescription: d\n---\nbody").is_none());
    // Invalid name.
    assert!(parse_skill_file("---\nname: Demo\ndescription: d\n---\nbody").is_none());
}

#[test]
fn unknown_keys_are_ignored() {
    let parsed =
        parse_skill_file("---\nname: demo\ndescription: d\nwhenToUse: later\ncustom: x\n---\nbody")
            .unwrap();
    assert_eq!(parsed.name, "demo");
}

#[test]
fn skill_names_descriptions_and_catalog_sources_are_bounded() {
    assert!(is_skill_name("a-1"));
    for invalid in ["", "A", "-a", "a-", "a--b", "a_b"] {
        assert!(!is_skill_name(invalid));
    }
    let long_desc = "x".repeat(MAX_SKILL_DESCRIPTION_CHARS + 8);
    let normalized = normalize_description(&long_desc);
    assert_eq!(normalized.chars().count(), MAX_SKILL_DESCRIPTION_CHARS);
    assert!(normalized.ends_with("..."));
}

#[test]
fn description_is_escaped_and_whitespace_collapsed() {
    let normalized = normalize_description("a  <b> & c");
    assert_eq!(normalized, "a &lt;b&gt; &amp; c");
}
