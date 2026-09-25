//! `[[subagent.providers]]` — parse, defaults, validation, merge-boundary trim.
//!
//! `Config::load` reads `./.oi/config.toml` against the *process* cwd, so the
//! cases that switch directories serialize on the same lock as
//! `mcp_config_validate.rs`: a sibling test's `set_current_dir` would land the
//! loader in a directory that never held this case's entry.

/// Serializes the cwd-switching cases. Process-global cwd, per
/// `mcp_config_validate.rs`; a poisoned lock is recovered rather than
/// unwrapped so one failing assertion cannot turn its siblings into opaque
/// panics.
fn cwd_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn load(body: &str) -> Result<config::Config, config::ConfigError> {
    let _serialized = cwd_lock();
    let dir = tempfile::tempdir().expect("tempdir");
    let oi_dir = dir.path().join(".oi");
    std::fs::create_dir_all(&oi_dir).expect("create oi dir");
    std::fs::write(oi_dir.join("config.toml"), body).expect("write config");

    let prev = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(dir.path()).expect("enter temp cwd");
    // `load` resolves `.oi/config.toml` against the process cwd; leaving the
    // temp dir set would break whichever case runs next.
    let result = config::Config::load();
    std::env::set_current_dir(&prev).expect("restore cwd");
    result
}

/// The invalid-config message, when there is one.
fn invalid_message(err: &config::ConfigError) -> &str {
    match err {
        config::ConfigError::Invalid { message, .. } => message,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

const FULL_ENTRY: &str = concat!(
    "[[subagent.providers]]\n",
    "name = \"remote\"\n",
    "command = \"/usr/local/bin/agent\"\n",
    "args = [\"--workspace\", \".\"]\n",
    "env = { KEY = \"value\" }\n",
    "cwd = \"/tmp/work\"\n",
    "permission = \"allow\"\n",
    "dispose_grace_ms = 1500\n",
    "dispose_eof_grace_ms = 2500\n",
);

#[test]
fn full_entry_parses_every_field() {
    let cfg = load(FULL_ENTRY).expect("valid config");

    let providers = &cfg.subagent_providers;
    assert_eq!(providers.len(), 1);
    let p = &providers[0];
    assert_eq!(p.name, "remote");
    assert_eq!(p.command, "/usr/local/bin/agent");
    assert_eq!(p.args, vec!["--workspace", "."]);
    assert_eq!(p.env.get("KEY").map(String::as_str), Some("value"));
    assert_eq!(p.cwd.as_deref(), Some("/tmp/work"));
    assert_eq!(p.permission, config::SubagentPermission::Allow);
    assert_eq!(p.dispose_grace_ms, Some(1500));
    assert_eq!(p.dispose_eof_grace_ms, Some(2500));
}

#[test]
fn absent_fields_default() {
    let cfg = load("[[subagent.providers]]\nname = \"bare\"\ncommand = \"agent\"\n")
        .expect("name and command are all an entry needs");
    let p = &cfg.subagent_providers[0];
    assert!(p.args.is_empty());
    assert!(p.env.is_empty());
    assert_eq!(p.cwd, None);
    // Default policy is reject: an unattended child must not be granted
    // permissions the parent never vetted.
    assert_eq!(p.permission, config::SubagentPermission::Reject);
    assert_eq!(p.dispose_grace_ms, None);
    assert_eq!(p.dispose_eof_grace_ms, None);
}

#[test]
fn no_section_means_no_providers() {
    let cfg = load("[daemon]\ncwd = \"/tmp\"\n").expect("no subagent section");
    assert!(
        cfg.subagent_providers.is_empty(),
        "opt-in: nothing is spawned"
    );
}

#[test]
fn name_fork_is_reserved() {
    let err = load("[[subagent.providers]]\nname = \"fork\"\ncommand = \"agent\"\n")
        .expect_err("fork is built-in");
    let msg = invalid_message(&err);
    assert!(
        msg.contains("fork"),
        "message must name the collision: {msg}"
    );
}

#[test]
fn empty_name_is_rejected() {
    let err = load("[[subagent.providers]]\nname = \" \"\ncommand = \"agent\"\n")
        .expect_err("whitespace-only name is empty after trim");
    let msg = invalid_message(&err);
    assert!(msg.contains("name"), "message must name the field: {msg}");
}

#[test]
fn empty_command_is_rejected() {
    let err = load("[[subagent.providers]]\nname = \"remote\"\ncommand = \" \"\n")
        .expect_err("a spawned provider needs something to spawn");
    let msg = invalid_message(&err);
    assert!(
        msg.contains("command"),
        "message must say what is missing: {msg}"
    );
}

#[test]
fn merge_trims_name_and_command() {
    // Trim happens at the merge boundary, not in validate: validate only
    // *tests* the trimmed value, so a padded command would pass validation
    // and still spawn with its spaces.
    let cfg = load(
        "[[subagent.providers]]\nname = \" remote \"\ncommand = \" /usr/bin/agent \"\ncwd = \" /tmp \"\n",
    )
    .expect("padded values are trimmed, not rejected");
    let p = &cfg.subagent_providers[0];
    assert_eq!(p.name, "remote");
    assert_eq!(p.command, "/usr/bin/agent");
    assert_eq!(p.cwd.as_deref(), Some("/tmp"));
}

#[test]
fn invalid_permission_value_is_rejected_by_serde() {
    // The enum deserializes lowercase; a typo is a TOML error, not a silent
    // default to reject.
    let err =
        load("[[subagent.providers]]\nname = \"r\"\ncommand = \"a\"\npermission = \"auto\"\n");
    assert!(
        err.is_err(),
        "unknown permission must not deserialize: {err:?}"
    );
}
