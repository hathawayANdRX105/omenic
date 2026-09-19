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

/// Write a `.oi/config.toml` with one `[[mcp.servers]]` entry and load it
/// from that cwd, mirroring the `Config::load` harness of
/// `llm_fallbacks_parse.rs`.
fn load_with_mcp_server(toml_entry: &str) -> Result<config::Config, config::ConfigError> {
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
    let result = std::panic::catch_unwind(config::Config::load);
    let _ = std::env::set_current_dir(&original_cwd);

    let loaded = match result {
        Ok(res) => res,
        Err(panic) => std::panic::resume_unwind(panic),
    };
    // Assert before dropping the tempdir so paths stay alive on failure.
    let config = loaded?;
    assert_eq!(
        config.mcp_servers.len(),
        1,
        "the [[mcp.servers]] entry must survive the load"
    );
    Ok(config)
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
    assert_eq!(cfg.mcp_servers[0].name, "both");
    assert_eq!(cfg.mcp_servers[0].command.as_deref(), Some("npx"));
    assert_eq!(
        cfg.mcp_servers[0].url.as_deref(),
        Some("http://127.0.0.1:9100/mcp")
    );
}

/// A `url` that does not start with `http(s)://` still loads.
#[test]
fn bad_url_scheme_still_loads() {
    let cfg = load_with_mcp_server("name = \"bad-scheme\"\nurl = \"127.0.0.1:9100/mcp\"\n")
        .expect("bad-scheme url config must load");
    assert_eq!(cfg.mcp_servers[0].name, "bad-scheme");
    assert_eq!(
        cfg.mcp_servers[0].url.as_deref(),
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
    assert_eq!(cfg.mcp_servers[0].name, "padded", "name is trimmed");
    assert_eq!(
        cfg.mcp_servers[0].command.as_deref(),
        Some("npx"),
        "command is trimmed at the merge boundary"
    );
}
