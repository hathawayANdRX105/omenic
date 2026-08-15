//! Smoke test: `load_yaml` loads the real spec file from .githooks/spec/ and
//! the `required_headings` sequence has the expected length (6).

use gate_core::shared::load_yaml;

#[test]
fn loads_real_spec_and_counts_required_headings() {
    let path = "/home/hathaway/projects/omenic/.githooks/spec/github_issues.yaml";
    let v = load_yaml(path).expect("spec yaml must parse");
    let headings = v
        .get("required_headings")
        .expect("required_headings key present")
        .as_sequence()
        .expect("required_headings is a sequence");
    assert_eq!(headings.len(), 6, "expected 6 required headings");
    // Spot-check a couple of entries by string value.
    let names: Vec<&str> = headings.iter().filter_map(|h| h.as_str()).collect();
    assert!(names.contains(&"Goal"));
    assert!(names.contains(&"Out of scope"));
}
