//! Integration tests for built-in tools.

use std::sync::atomic::AtomicBool;

use serde_json::{Value, json};
use tools::bash::RunBash;
use tools::delete::DeleteFile;
use tools::edit::EditFile;
use tools::glob::Glob;
use tools::grep::Grep;
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

#[test]
fn grep_finds_pattern_in_temp_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.txt");
    std::fs::write(&path, "hello world\nfoo bar\nhello again\n").unwrap();
    let s = sig();
    let out = Grep
        .execute(
            &json!({"pattern": "hello", "path": path.to_str().unwrap()}),
            &s,
        )
        .unwrap();
    assert!(out.contains("hello world"), "should match line 1");
    assert!(out.contains("hello again"), "should match line 3");
    assert!(
        !out.contains("foo bar"),
        "should not match non-pattern line"
    );
}

#[test]
fn grep_no_matches_returns_message() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.txt");
    std::fs::write(&path, "nothing here\n").unwrap();
    let s = sig();
    let out = Grep
        .execute(
            &json!({"pattern": "nonexistent", "path": path.to_str().unwrap()}),
            &s,
        )
        .unwrap();
    assert_eq!(out, "no matches found");
}

#[test]
fn glob_lists_matching_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "").unwrap();
    std::fs::write(dir.path().join("b.rs"), "").unwrap();
    std::fs::write(dir.path().join("c.txt"), "").unwrap();
    let s = sig();
    let out = Glob
        .execute(
            &json!({"pattern": "*.rs", "path": dir.path().to_str().unwrap()}),
            &s,
        )
        .unwrap();
    assert!(out.contains("a.rs"), "should list a.rs");
    assert!(out.contains("b.rs"), "should list b.rs");
    assert!(!out.contains("c.txt"), "should not list c.txt");
}

#[test]
fn glob_no_matches_returns_message() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "").unwrap();
    let s = sig();
    let out = Glob
        .execute(
            &json!({"pattern": "*.nonexistent", "path": dir.path().to_str().unwrap()}),
            &s,
        )
        .unwrap();
    assert_eq!(out, "no files matched");
}

#[test]
fn delete_file_removes_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("to_delete.txt");
    std::fs::write(&path, "bye").unwrap();
    assert!(path.exists());
    let s = sig();
    let out = DeleteFile
        .execute(&json!({"path": path.to_str().unwrap()}), &s)
        .unwrap();
    assert!(
        out.contains("trash") || out.contains("deleted"),
        "should report removal"
    );
    assert!(!path.exists(), "file should be gone");
}

#[test]
fn delete_nonexistent_file_errors() {
    let s = sig();
    let err = DeleteFile
        .execute(&json!({"path": "/tmp/oi-nonexistent-12345.txt"}), &s)
        .unwrap_err();
    assert!(err.to_string().contains("does not exist"));
}
