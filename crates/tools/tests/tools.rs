//! Integration tests for built-in tools.

use std::sync::atomic::AtomicBool;

use serde_json::{Value, json};
use tools::bash::RunBash;
use tools::edit::EditFile;
use tools::write::WriteFile;
use tools::{Tool, builtin_tools, def};

fn sig() -> AtomicBool {
    AtomicBool::new(false)
}

#[test]
fn truncation_keeps_tail_and_spills_full_output() {
    let content: String = (0..250).map(|i| format!("line{i}\n")).collect();
    let out = tools::truncate_output(content.trim_end(), 7).unwrap();
    assert!(out.starts_with("[output truncated: showing last 200 of 250 lines."));
    assert!(out.contains("full output: /tmp/oi-output-7.txt"));
    assert!(out.trim_end().ends_with("line249"));
    let full = std::fs::read_to_string("/tmp/oi-output-7.txt").unwrap();
    assert!(full.starts_with("line0\n"));
}

#[test]
fn short_output_not_truncated() {
    assert_eq!(tools::truncate_output("a\nb", 0).unwrap(), "a\nb");
}

#[test]
fn edit_uniqueness_enforced() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f.txt");
    std::fs::write(&path, "alpha beta alpha").unwrap();
    let sig = sig();

    // No match.
    let err = EditFile
        .execute(
            &json!({"path": path, "old_string": "zzz", "new_string": "x"}),
            &sig,
        )
        .unwrap_err();
    assert!(err.to_string().contains("not found"));

    // Ambiguous.
    let err = EditFile
        .execute(
            &json!({"path": path, "old_string": "alpha", "new_string": "x"}),
            &sig,
        )
        .unwrap_err();
    assert!(err.to_string().contains("matches 2 places"));

    // Unique replace works, including $-literals staying literal.
    EditFile
        .execute(
            &json!({"path": path, "old_string": "beta", "new_string": "$& $1"}),
            &sig,
        )
        .unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "alpha $& $1 alpha");
}

#[test]
fn write_creates_parents_and_reports() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a/b/c.txt");
    let sig = sig();
    let out = WriteFile
        .execute(&json!({"path": path, "content": "hello"}), &sig)
        .unwrap();
    assert_eq!(out, format!("wrote {} (5 chars)", path.display()));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "hello");
}

#[test]
fn run_bash_captures_output_and_exit_codes() {
    let sig = sig();
    let ok = RunBash
        .execute(&json!({"command": "echo hi"}), &sig)
        .unwrap();
    assert_eq!(ok.trim(), "hi");

    let fail = RunBash
        .execute(&json!({"command": "echo bad >&2; exit 3"}), &sig)
        .unwrap();
    assert!(fail.contains("[exit 3]"), "got: {fail}");
    assert!(fail.contains("bad"));
}

#[test]
fn run_bash_respects_abort_signal() {
    let sig = AtomicBool::new(true);
    let out = RunBash
        .execute(&json!({"command": "echo nope"}), &sig)
        .unwrap();
    assert_eq!(out, "aborted");
}

#[test]
fn missing_args_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f.txt");
    std::fs::write(&path, "hello").unwrap();
    let sig = sig();
    let err = tools::read::ReadFile.execute(&json!({}), &sig).unwrap_err();
    assert!(err.to_string().contains("missing string argument: path"));
}

#[test]
fn defs_expose_schema_and_names() {
    let tools = builtin_tools();
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert_eq!(
        names,
        [
            "read_file",
            "write_file",
            "edit",
            "run_bash",
            "grep",
            "glob",
            "delete_file"
        ]
    );
    for t in &tools {
        let d = def(t.as_ref());
        assert!(!d.description.is_empty());
        assert_eq!(d.parameters["type"], "object");
    }
}
