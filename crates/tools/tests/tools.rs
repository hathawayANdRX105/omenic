//! Integration tests for built-in tools.

use std::sync::atomic::AtomicBool;

use serde_json::{Value, json};
use tools::bash::RunBash;
use tools::delete::DeleteFile;
use tools::edit::EditFile;
use tools::glob::Glob;
use tools::grep::Grep;
use tools::write::WriteFile;
use tools::{Decision, Policy, Rule, Tool, builtin_tools, builtin_tools_with_policy, def};

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
    let names: Vec<String> = tools.iter().map(|t| t.name().to_string()).collect();
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

/// Pull one registered tool out of a policy-wired registry: proves the
/// registry actually applies the policy, not just that `Guarded` can.
fn registered(policy: Policy, name: &str) -> Box<dyn Tool> {
    builtin_tools_with_policy(policy)
        .into_iter()
        .find(|t| t.name() == name)
        .expect("tool should be registered")
}

#[test]
fn permission_policy_allows_denies_and_reports_tool_and_reason() {
    let policy = Policy::allow_all()
        .rule(Rule::deny("run_bash", "rm -rf", "destructive command"))
        .rule(Rule::deny("delete_file", "src/", "protected tree"));

    // allow: no rule matches, default permits
    assert!(policy.check("run_bash", "ls -la").is_ok());
    // allow: rules are tool-scoped, so the same text under another tool passes
    assert!(policy.check("write_file", "rm -rf").is_ok());

    // deny: message names the blocked tool and the matched rule's reason
    assert_eq!(
        policy
            .check("run_bash", "rm -rf /")
            .unwrap_err()
            .to_string(),
        "run_bash denied by permission policy: destructive command"
    );
    assert_eq!(
        policy
            .check("delete_file", "crates/tools/src/lib.rs")
            .unwrap_err()
            .to_string(),
        "delete_file denied by permission policy: protected tree"
    );

    // first matching rule wins over a later contradicting one
    let ordered = Policy::deny_all()
        .rule(Rule::allow("edit", "workspace/", "inside workspace"))
        .rule(Rule::deny("edit", "", "everything else"));
    assert!(ordered.check("edit", "workspace/a.rs").is_ok());
    assert_eq!(
        ordered
            .check("edit", "/etc/passwd")
            .unwrap_err()
            .to_string(),
        "edit denied by permission policy: everything else"
    );
}

#[test]
fn permission_default_governs_unmatched_invocations() {
    // deny-all default: an unmatched tool/subject is rejected with the
    // fallback reason, and matching an unrelated rule does not help.
    let deny = Policy::deny_all().rule(Rule::allow("write_file", "allowed", "opt-in"));
    assert_eq!(
        deny.check("run_bash", "echo hi").unwrap_err().to_string(),
        "run_bash denied by permission policy: no matching rule"
    );
    assert_eq!(
        deny.check("write_file", "other.txt")
            .unwrap_err()
            .to_string(),
        "write_file denied by permission policy: no matching rule"
    );
    assert!(deny.check("write_file", "allowed.txt").is_ok());

    // allow-all default: unknown tool names and arbitrary subjects pass
    let allow = Policy::allow_all();
    assert_eq!(allow.default, Decision::Allow);
    assert!(allow.check("some_unknown_tool", "anything").is_ok());
    assert_eq!(Policy::deny_all().default, Decision::Deny);

    // registry keeps identical names and order under any policy
    let guarded: Vec<String> = builtin_tools_with_policy(Policy::deny_all())
        .iter()
        .map(|t| t.name().to_string())
        .collect();
    let unrestricted: Vec<String> = builtin_tools()
        .iter()
        .map(|t| t.name().to_string())
        .collect();
    assert_eq!(guarded, unrestricted);
}

#[test]
fn permission_deny_all_covers_read_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("readable.txt");
    std::fs::write(&path, "visible").unwrap();

    let err = registered(Policy::deny_all(), "read_file")
        .execute(&json!({"path": path}), &sig())
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "read_file denied by permission policy: no matching rule"
    );

    // the same registry permits the read once a rule opts it in
    let policy = Policy::deny_all().rule(Rule::allow("read_file", "readable", "opt-in"));
    let out = registered(policy, "read_file")
        .execute(&json!({"path": path}), &sig())
        .unwrap();
    assert!(
        out.contains("visible"),
        "allowed read should return content"
    );
}

#[test]
fn permission_denied_write_leaves_target_absent() {
    let dir = tempfile::tempdir().unwrap();
    let blocked = dir.path().join("blocked.txt");
    let allowed = dir.path().join("allowed.txt");
    let policy = Policy::allow_all().rule(Rule::deny("write_file", "blocked", "protected name"));
    let write = registered(policy, "write_file");
    let s = sig();

    let err = write
        .execute(&json!({"path": blocked, "content": "nope"}), &s)
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "write_file denied by permission policy: protected name"
    );
    assert!(!blocked.exists(), "denied write must not create the file");

    // same tool, allowed subject: the write still happens
    write
        .execute(&json!({"path": allowed, "content": "yes"}), &s)
        .unwrap();
    assert_eq!(std::fs::read_to_string(&allowed).unwrap(), "yes");
}

#[test]
fn permission_denied_bash_creates_no_marker() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("marker.txt");
    let command = format!("printf touched > {}", marker.display());
    let s = sig();

    // baseline: the command really does create the marker when allowed
    registered(Policy::allow_all(), "run_bash")
        .execute(&json!({"command": command}), &s)
        .unwrap();
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "touched");
    std::fs::remove_file(&marker).unwrap();

    let policy = Policy::allow_all().rule(Rule::deny("run_bash", "printf", "no side effects"));
    let err = registered(policy, "run_bash")
        .execute(&json!({"command": command}), &s)
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "run_bash denied by permission policy: no side effects"
    );
    assert!(
        !marker.exists(),
        "denied run_bash must not spawn the subprocess"
    );
}

#[test]
fn permission_denied_edit_and_delete_leave_file_intact() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("guarded.txt");
    std::fs::write(&path, "keep me").unwrap();
    let policy = Policy::deny_all();
    let s = sig();

    let err = registered(policy.clone(), "edit")
        .execute(
            &json!({"path": path, "old_string": "keep me", "new_string": "clobbered"}),
            &s,
        )
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "edit denied by permission policy: no matching rule"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep me");

    let err = registered(policy, "delete_file")
        .execute(&json!({"path": path}), &s)
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "delete_file denied by permission policy: no matching rule"
    );
    assert!(path.exists(), "denied delete must leave the file in place");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep me");
}

#[test]
fn permission_judges_absent_subject_instead_of_skipping() {
    let s = sig();

    // grep/glob have no path argument here, so they are judged on the "."
    // they would default to searching.
    let policy = Policy::allow_all().rule(Rule::deny("grep", ".", "cwd is off limits"));
    let err = registered(policy, "grep")
        .execute(&json!({"pattern": "x"}), &s)
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "grep denied by permission policy: cwd is off limits"
    );

    // fail closed: omitting the subject entirely must not sidestep a deny
    // default, and an allow rule keyed on a real command does not match "".
    let policy = Policy::deny_all().rule(Rule::allow("run_bash", "ls", "safe listing"));
    let err = registered(policy.clone(), "run_bash")
        .execute(&json!({}), &s)
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "run_bash denied by permission policy: no matching rule"
    );
    // the same policy still permits the command it named
    let out = registered(policy, "run_bash")
        .execute(&json!({"command": "ls"}), &s)
        .unwrap();
    assert!(!out.contains("denied"), "allowed command should run: {out}");

    // under an allow default the guard is transparent, so a missing argument
    // still surfaces the inner tool's own validation error.
    let err = registered(Policy::allow_all(), "write_file")
        .execute(&json!({}), &s)
        .unwrap_err();
    assert_eq!(err.to_string(), "missing string argument: path");
}
