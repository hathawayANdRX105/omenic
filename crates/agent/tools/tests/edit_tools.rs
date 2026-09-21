//! Tests for apply_patch and str_replace_editor tools.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use serde_json::{Value, json};
use tools::apply_patch::ApplyPatch;
use tools::str_replace_editor::StrReplaceEditor;
use tools::{Tool, ToolError};

fn temp_dir(test_name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("omenic_test_{}_{}", test_name, std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn cleanup_temp(dir: &PathBuf) {
    let _ = fs::remove_dir_all(dir);
}

/// Build a test file with given content
fn write_test_file(dir: &PathBuf, name: &str, content: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, content).unwrap();
    path
}

#[test]
fn apply_patch_single_hunk() {
    let dir = temp_dir("apply_patch_single_hunk");
    let file = write_test_file(&dir, "test.txt", "line 1\nline 2\nline 3\n");

    let patch = r#"--- a/test.txt
+++ b/test.txt
@@ -1,3 +1,3 @@
 line 1
-line 2
+line 2 modified
 line 3
"#;

    let args = json!({
        "path": file.to_str().unwrap(),
        "patch": patch
    });

    let tool = ApplyPatch;
    let result = tool.execute(&args, &AtomicBool::new(false));
    assert!(result.is_ok(), "apply_patch failed: {:?}", result);

    let content = fs::read_to_string(&file).unwrap();
    assert_eq!(content, "line 1\nline 2 modified\nline 3\n");

    cleanup_temp(&dir);
}

#[test]
fn apply_patch_multi_hunk() {
    let dir = temp_dir("apply_patch_multi_hunk");
    let file = write_test_file(&dir, "test.txt", "line 1\nline 2\nline 3\nline 4\nline 5\n");

    let patch = r#"--- a/test.txt
+++ b/test.txt
@@ -1,3 +1,3 @@
 line 1
-line 2
+line 2 modified
 line 3
@@ -4,2 +4,2 @@
 line 4
-line 5
+line 5 modified
"#;

    let args = json!({
        "path": file.to_str().unwrap(),
        "patch": patch
    });

    let tool = ApplyPatch;
    let result = tool.execute(&args, &AtomicBool::new(false));
    assert!(result.is_ok(), "apply_patch failed: {:?}", result);

    let content = fs::read_to_string(&file).unwrap();
    assert_eq!(
        content,
        "line 1\nline 2 modified\nline 3\nline 4\nline 5 modified\n"
    );

    cleanup_temp(&dir);
}

#[test]
fn apply_patch_conflicting_hunks_rejected() {
    let dir = temp_dir("apply_patch_conflicting_hunks");
    let file = write_test_file(&dir, "test.txt", "line 1\nline 2\nline 3\n");

    // Two hunks that overlap - should fail
    let patch = r#"--- a/test.txt
+++ b/test.txt
@@ -1,3 +1,3 @@
 line 1
-line 2
+line 2 first
 line 3
@@ -1,3 +1,3 @@
 line 1
-line 2
+line 2 second
 line 3
"#;

    let args = json!({
        "path": file.to_str().unwrap(),
        "patch": patch
    });

    let tool = ApplyPatch;
    let result = tool.execute(&args, &AtomicBool::new(false));
    assert!(
        result.is_err(),
        "apply_patch should fail on conflicting hunks, got: {:?}",
        result
    );

    cleanup_temp(&dir);
}

#[test]
fn str_replace_editor_str_replace_unique() {
    let dir = temp_dir("str_replace_editor_str_replace_unique");
    let file = write_test_file(&dir, "test.txt", "hello world\nhello universe\n");

    let args = json!({
        "command": "str_replace",
        "path": file.to_str().unwrap(),
        "old_str": "hello world",
        "new_str": "hi world"
    });

    let tool = StrReplaceEditor;
    let result = tool.execute(&args, &AtomicBool::new(false));
    assert!(result.is_ok(), "str_replace failed: {:?}", result);

    let content = fs::read_to_string(&file).unwrap();
    assert_eq!(content, "hi world\nhello universe\n");

    cleanup_temp(&dir);
}

#[test]
fn str_replace_editor_old_str_not_found() {
    let dir = temp_dir("str_replace_editor_old_str_not_found");
    let file = write_test_file(&dir, "test.txt", "hello world\n");

    let args = json!({
        "command": "str_replace",
        "path": file.to_str().unwrap(),
        "old_str": "not found",
        "new_str": "replacement"
    });

    let tool = StrReplaceEditor;
    let result = tool.execute(&args, &AtomicBool::new(false));
    assert!(
        result.is_err(),
        "str_replace should fail when old_str not found"
    );
    match result {
        Err(ToolError::Message(msg)) => {
            assert!(
                msg.contains("not found"),
                "error should mention not found: {}",
                msg
            );
        }
        _ => panic!("expected ToolError::Message"),
    }

    cleanup_temp(&dir);
}

#[test]
fn str_replace_editor_old_str_not_unique() {
    let dir = temp_dir("str_replace_editor_old_str_not_unique");
    let file = write_test_file(&dir, "test.txt", "hello\nhello\n");

    let args = json!({
        "command": "str_replace",
        "path": file.to_str().unwrap(),
        "old_str": "hello",
        "new_str": "hi"
    });

    let tool = StrReplaceEditor;
    let result = tool.execute(&args, &AtomicBool::new(false));
    assert!(
        result.is_err(),
        "str_replace should fail when old_str matches multiple, got: {:?}",
        result
    );
    match result {
        Err(ToolError::Message(msg)) => {
            assert!(
                msg.contains("unique") || msg.contains("matches"),
                "error should mention uniqueness: {}",
                msg
            );
        }
        _ => panic!("expected ToolError::Message"),
    }

    cleanup_temp(&dir);
}

#[test]
fn str_replace_editor_view_file() {
    let dir = temp_dir("str_replace_editor_view_file");
    let file = write_test_file(&dir, "test.txt", "line 1\nline 2\nline 3\nline 4\nline 5\n");

    // View with range
    let args = json!({
        "command": "view",
        "path": file.to_str().unwrap(),
        "view_range": [2, 4]
    });

    let tool = StrReplaceEditor;
    let result = tool.execute(&args, &AtomicBool::new(false));
    assert!(result.is_ok(), "view failed: {:?}", result);

    let output = result.unwrap();
    assert!(output.contains("line 2"));
    assert!(output.contains("line 3"));
    assert!(output.contains("line 4"));
    // Should NOT contain line 1 or line 5
    assert!(!output.contains("line 1"));
    assert!(!output.contains("line 5"));

    cleanup_temp(&dir);
}

#[test]
fn str_replace_editor_create_file() {
    let dir = temp_dir("str_replace_editor_create_file");
    let file = dir.join("new_file.txt");

    let args = json!({
        "command": "create",
        "path": file.to_str().unwrap(),
        "new_str": "new content\n"
    });

    let tool = StrReplaceEditor;
    let result = tool.execute(&args, &AtomicBool::new(false));
    assert!(result.is_ok(), "create failed: {:?}", result);

    let content = fs::read_to_string(&file).unwrap();
    assert_eq!(content, "new content\n");

    // Try to create again - should fail
    let result2 = tool.execute(&args, &AtomicBool::new(false));
    assert!(result2.is_err(), "create should fail when file exists");

    cleanup_temp(&dir);
}

#[test]
fn str_replace_editor_path_escape_rejection() {
    // Test that path traversal attempts are rejected
    let dir = temp_dir("str_replace_editor_path_escape_rejection");

    // Try to write outside temp dir
    let args = json!({
        "command": "create",
        "path": "/etc/passwd",
        "new_str": "malicious"
    });

    let tool = StrReplaceEditor;
    let result = tool.execute(&args, &AtomicBool::new(false));
    // Should fail - either permission denied or path validation
    assert!(
        result.is_err(),
        "create should fail for system paths, got: {:?}",
        result
    );

    // Try normal file - should work
    let file = dir.join("test.txt");
    fs::write(&file, "original").unwrap();

    let args = json!({
        "command": "str_replace",
        "path": file.to_str().unwrap(),
        "old_str": "original",
        "new_str": "modified"
    });

    let result = tool.execute(&args, &AtomicBool::new(false));
    // This should succeed - it's a normal file
    assert!(result.is_ok(), "normal file should work: {:?}", result);

    cleanup_temp(&dir);
}
