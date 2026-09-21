//! End-to-end tests for `oi profile` (boot/bundle profiles).
//!
//! Real `oi` binary as a subprocess in a tempdir cwd; no daemon, no API keys
//! — `profile` only touches `.oi/config.toml`.

use std::process::{Command, Stdio};

use tempfile::TempDir;

fn run_oi(cwd: &std::path::Path, args: &[&str]) -> (String, String, bool) {
    let exe = env!("CARGO_BIN_EXE_oi");
    let output = Command::new(exe)
        .args(args)
        .current_dir(cwd)
        // Keep the ambient environment from steering the run: data dir and
        // socket live inside the tempdir.
        .env("OMENIC_DATA_DIR", cwd.join(".oi"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn oi");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    )
}

#[test]
fn profile_list_shows_both_profiles() {
    let dir = TempDir::new().unwrap();
    let (stdout, _stderr, ok) = run_oi(dir.path(), &["profile", "list"]);
    assert!(ok, "profile list must succeed");
    assert!(stdout.contains("boot"), "got: {stdout}");
    assert!(stdout.contains("bundle"), "got: {stdout}");
}

#[test]
fn profile_apply_writes_a_loadable_config() {
    let dir = TempDir::new().unwrap();
    let (stdout, _stderr, ok) = run_oi(dir.path(), &["profile", "apply", "boot"]);
    assert!(ok, "apply must succeed: {stdout}");
    let content = std::fs::read_to_string(dir.path().join(".oi/config.toml")).unwrap();
    assert!(content.contains("omp_path = \"omp\""));
    assert!(content.contains("[llm]"));
    assert!(content.contains("[daemon]"));
    // The written profile must be valid TOML with the sections a real run
    // reads — a profile that does not parse is not a starting point.
    let table: toml::Table = toml::from_str(&content).expect("profile TOML must parse");
    assert!(table.contains_key("llm"));
    assert!(table.contains_key("daemon"));
}

#[test]
fn profile_apply_refuses_to_overwrite_existing_config() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".oi")).unwrap();
    std::fs::write(dir.path().join(".oi/config.toml"), "model = \"mine\"\n").unwrap();
    let (_stdout, stderr, ok) = run_oi(dir.path(), &["profile", "apply", "boot"]);
    assert!(!ok, "apply over an existing config must fail");
    assert!(stderr.contains("refusing to overwrite"), "got: {stderr}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".oi/config.toml")).unwrap(),
        "model = \"mine\"\n"
    );
}

#[test]
fn profile_apply_unknown_name_lists_available() {
    let dir = TempDir::new().unwrap();
    let (_stdout, stderr, ok) = run_oi(dir.path(), &["profile", "apply", "nope"]);
    assert!(!ok, "unknown profile must fail");
    assert!(stderr.contains("unknown profile"), "got: {stderr}");
    assert!(stderr.contains("boot"), "got: {stderr}");
    assert!(stderr.contains("bundle"), "got: {stderr}");
    assert!(!dir.path().join(".oi/config.toml").exists());
}
