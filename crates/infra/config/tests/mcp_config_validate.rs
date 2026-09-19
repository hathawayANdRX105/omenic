//! Finding 4 (B1/B2 review) — the config loader's MCP `validate` warnings.
//!
//! `Config::validate` warns (stderr) but never rejects:
//!
//! 1. a server with **both** `url` and `command` — the mcp crate prefers the
//!    HTTP transport and silently ignores `command`;
//! 2. a `url` that does not start with `http(s)://` — the HTTP transport
//!    cannot dial it.
//!
//! The behavioural pin here is "still loads": the warnings go to stderr and
//! are not asserted, only the `Ok` outcome is.
//!
//! **These cases share one process-wide current directory.** `Config::load`
//! reads `./.oi/config.toml` — resolved against the *process* cwd, since it
//! takes no directory argument — and `set_current_dir` is a property of the
//! process, not of a test. Three cases that each move the cwd and then load
//! therefore race each other when cargo runs them in parallel threads: the
//! writer's `set_current_dir` can be overwritten by a sibling's before its
//! own `load` reads, and the loader then opens a directory that never had
//! this case's entry. That is a defect of this harness, not of the loader,
//! so the cases serialize on a lock and each restores the cwd it found.

/// Serializes the cwd-switching cases below. The `set_current_dir` in
/// `load_with_mcp_server` is process-global, so two cases that overlap can
/// land the loader in a directory that is not the writer's.
///
/// A poisoned lock is taken rather than unwrapped: the cases assert on
/// config content, and one failing assertion must not turn the other two
/// into "poisoned lock" panics that say nothing about the config.
fn cwd_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Write a `.oi/config.toml` with one `[[mcp.servers]]` entry and load it
/// from that cwd, mirroring the `Config::load` harness of
/// `llm_fallbacks_parse.rs`.
///
/// Holding `cwd_lock` for the whole body, restore included, is what makes
/// the pair (write, load) atomic with respect to the sibling cases.
fn load_with_mcp_server(toml_entry: &str) -> Result<config::Config, config::ConfigError> {
    let _serialized = cwd_lock();
    let dir = tempfile::tempdir().expect("tempdir");
    let oi_dir = dir.path().join(".oi");
    std::fs::create_dir_all(&oi_dir).expect("create .oi dir");
    std::fs::write(
        oi_dir.join("config.toml"),
        format!("[[mcp.servers]]\n{toml_entry}"),
    )
    .expect("write config.toml");

    let original_cwd = std::env::current_dir().expect("current_dir");
    std::env::set_current_dir(dir.path()).expect("switch cwd");
    // `catch_unwind` keeps the restore unconditional: `Config::load` is not
    // expected to panic, but a panic here would otherwise leave the process
    // inside a tempdir that `dir`'s drop is about to delete.
    let result = std::panic::catch_unwind(config::Config::load);
    std::env::set_current_dir(&original_cwd).expect("restore cwd");

    match result {
        Ok(res) => res,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

/// The entry this helper wrote, picked by name so a neighbouring tempdir's
/// entry (see the note above) cannot shift the index.
fn loaded_server<'a>(cfg: &'a config::Config, name: &str) -> &'a config::McpServerConfig {
    cfg.mcp_servers
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| {
            panic!(
                "server `{name}` did not survive the load; loaded: {:?}",
                cfg.mcp_servers.iter().map(|s| &s.name).collect::<Vec<_>>()
            )
        })
}

/// A server with both `url` and `command` loads fine: `validate` only warns
/// (on stderr) that the HTTP transport wins and `command` is ignored — it
/// must not reject the legacy-shaped config.
#[test]
fn both_url_and_command_still_loads() {
    let cfg = load_with_mcp_server(
        "name = \"both\"\ncommand = \"npx\"\nurl = \"http://127.0.0.1:9100/mcp\"\n",
    )
    .expect("both-set config must load");
    let s = loaded_server(&cfg, "both");
    assert_eq!(s.command.as_deref(), Some("npx"));
    assert_eq!(s.url.as_deref(), Some("http://127.0.0.1:9100/mcp"));
}

/// A `url` that does not start with `http(s)://` still loads.
#[test]
fn bad_url_scheme_still_loads() {
    let cfg = load_with_mcp_server("name = \"bad-scheme\"\nurl = \"127.0.0.1:9100/mcp\"\n")
        .expect("bad-scheme url config must load");
    assert_eq!(
        loaded_server(&cfg, "bad-scheme").url.as_deref(),
        Some("127.0.0.1:9100/mcp")
    );
}

/// Padded transport fields are trimmed while the config is merged.
///
/// `validate` trims only to *test* a value, so without normalizing at the
/// merge boundary a `command = " npx "` passes validation and is then handed
/// to `Command::new` with its spaces — which cannot spawn.
#[test]
fn padded_transport_fields_are_trimmed() {
    let cfg = load_with_mcp_server("name = \" padded \"\ncommand = \" npx \"\n")
        .expect("padded command config must load");
    assert_eq!(
        loaded_server(&cfg, "padded").command.as_deref(),
        Some("npx"),
        "name and command are trimmed at the merge boundary"
    );
}
