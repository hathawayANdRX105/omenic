use task::Task;
use task::store::Store;
use task::template::apply;
use tempfile::tempdir;

/// A template with no explicit `deps` must still get a sibling chain
/// (b→a, c→b) and terminal aggregation (phase→c). Guards the derivation
/// path that templates with explicit deps never exercise.
#[test]
fn apply_derives_sibling_chain_and_terminal_aggregation() {
    let tmp = tempdir().unwrap();
    let store = Store::new(tmp.path());
    let no_deps = r#"tasks:
  - key: a
    title: "a"
    kind: task
    description: "a"
    acceptance: "a"
  - key: b
    title: "b"
    kind: task
    description: "b"
    acceptance: "b"
  - key: c
    title: "c"
    kind: task
    description: "c"
    acceptance: "c"
deps: []
"#;
    let tdir = tmp.path().join("templates").join("phases");
    std::fs::create_dir_all(&tdir).unwrap();
    std::fs::write(tdir.join("chain.yaml"), no_deps).unwrap();

    apply(&store, tmp.path(), "chain", "t-x", None).unwrap();
    let map: std::collections::HashMap<String, Task> = store
        .load_all()
        .unwrap()
        .into_iter()
        .map(|t| (t.id.clone(), t))
        .collect();

    assert_eq!(map["t-x-chain-a"].deps, Vec::<String>::new());
    assert_eq!(map["t-x-chain-b"].deps, vec!["t-x-chain-a".to_string()]);
    assert_eq!(map["t-x-chain-c"].deps, vec!["t-x-chain-b".to_string()]);
    assert_eq!(map["t-x-chain"].deps, vec!["t-x-chain-c".to_string()]);
}
