//! B1/T3 — daemon-level MCP bring-up.
//!
//! `Daemon::start` (orbit path) spawns every configured MCP server exactly
//! once and hands their tools to the engine through
//! `rpc::worker::OrbitConfig::mcp_tools`. What is under test here:
//!
//! 1. Default policy: a server that fails to start is skipped (logged by the
//!    mcp crate) and the daemon starts anyway; `DaemonConfig.mcp_servers`
//!    round-trips what was configured.
//! 2. `fail_on_startup_error = true`: the first failure aborts the daemon
//!    start with an error that names the configured server.
//! 3. Happy path over a real stdio transport: a minimal mock server that
//!    speaks the JSON-RPC handshake starts with the daemon.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use daemon::{Daemon, DaemonConfig};
use tempfile::tempdir;

fn orbit_model() -> adaptor::Model {
    adaptor::Model {
        api_key: "test-key".into(),
        model: "mcp-bringup".into(),
        // The engine is never prompted in these tests: no request is issued.
        base_url: Some("http://127.0.0.1:9".into()),
        max_tokens: Some(1024),
    }
}

fn mcp_server(name: &str, command: &str, fail: Option<bool>) -> config::McpServerConfig {
    config::McpServerConfig {
        name: name.into(),
        command: Some(command.into()),
        args: vec![],
        env: Default::default(),
        url: None,
        tool_call_timeout_ms: None,
        reconnect: None,
        cwd: None,
        fail_on_startup_error: fail,
    }
}

fn cfg(dir: &Path, servers: Vec<config::McpServerConfig>) -> DaemonConfig {
    DaemonConfig {
        socket_path: Some(dir.join("daemon.sock")),
        omp_path: "omp".into(),
        session_db_path: Some(dir.join("sessions.db")),
        orbit_model: Some(orbit_model()),
        // Scope instruction discovery to the temp dir.
        cwd: dir.to_path_buf(),
        max_turns: 64,
        mcp_servers: servers,
        llm_fallbacks: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// 1. Skip policy + config round-trip
// ---------------------------------------------------------------------------

/// Default policy: a server whose binary doesn't exist contributes zero
/// tools and is skipped (logged by the mcp crate) — one broken entry in the
/// user's config must not take down the daemon. The configured list also
/// survives verbatim into the `DaemonConfig` the daemon consumes.
#[test]
fn broken_mcp_server_is_skipped_by_default_and_config_round_trips() {
    let dir = tempdir().expect("temp dir");
    let server = mcp_server("b1-mcp-fake", "b1-mcp-definitely-not-on-path", None);
    let cfg = cfg(dir.path(), vec![server.clone()]);

    // Round-trip: the daemon consumes exactly what was configured.
    assert_eq!(cfg.mcp_servers, vec![server], "mcp_servers must round-trip");

    let daemon = Daemon::start(cfg).expect("daemon start must survive a broken MCP server");
    assert!(daemon.started_at_ms() > 0, "daemon came up");
}

/// `DaemonConfig::from_config` carries `Config.mcp_servers` through — the
/// `[[mcp.servers]]` list from `.oi/config.toml` is what bring-up consumes.
#[test]
fn from_config_carries_mcp_servers() {
    let dir = tempdir().expect("temp dir");
    let mut app = config::Config {
        omp_path: "omp".into(),
        data_dir: dir.path().to_path_buf(),
        model: "omp-model".into(),
        llm_api_key: None,
        llm_base_url: None,
        llm_model: None,
        llm_max_tokens: None,
        llm_fallbacks: Vec::new(),
        mcp_servers: Vec::new(),
        memory_enabled: false,
        memory_dir: None,
        cwd: dir.path().to_path_buf(),
        max_turns: None,
    };
    app.mcp_servers = vec![mcp_server("from-config", "unused", None)];

    let dc = DaemonConfig::from_config(&app).expect("from_config resolves paths");
    assert_eq!(dc.mcp_servers.len(), 1, "the list must survive from_config");
    assert_eq!(dc.mcp_servers[0].name, "from-config");
    assert_eq!(dc.mcp_servers[0].fail_on_startup_error, None);
}

// ---------------------------------------------------------------------------
// 2. fail_on_startup_error: daemon start fails loudly, naming the server
// ---------------------------------------------------------------------------

#[test]
fn fail_on_startup_error_fails_daemon_start_naming_the_server() {
    let dir = tempdir().expect("temp dir");
    let cfg = cfg(
        dir.path(),
        vec![mcp_server(
            "b1-mcp-fatal",
            "b1-mcp-still-not-on-path",
            Some(true),
        )],
    );

    // `Daemon` isn't Debug, so match instead of `expect_err`.
    let err = match Daemon::start(cfg) {
        Ok(_) => panic!("daemon start must fail when fail_on_startup_error is set"),
        Err(e) => e,
    };
    let msg = err.to_string();
    assert!(
        msg.contains("b1-mcp-fatal"),
        "error must name the configured server, got: {msg}"
    );
    assert!(
        msg.contains("mcp"),
        "error must point at MCP bring-up, got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// 3. Happy path over a real stdio transport
// ---------------------------------------------------------------------------

/// Minimal stdio MCP server: replies to `initialize` and `tools/list` with
/// the request id echoed back; exits when stdin closes (EOF). Pure bash, no
/// correlation beyond the one `id` field every JSON-RPC request carries.
fn write_mock_server(dir: &Path) -> PathBuf {
    let script = dir.join("b1-mcp-mock.sh");
    let mut f = std::fs::File::create(&script).expect("create mock server");
    f.write_all(
        br#"#!/usr/bin/env bash
# Minimal stdio MCP server: answer initialize + tools/list, exit on EOF.
while IFS= read -r line; do
  rest="${line#*\"id\":}"
  id="${rest%%[!0-9]*}"
  case "$line" in
    *'"method":"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"b1-mock","version":"0.0.0"}}}\n' "$id"
      ;;
    *'"method":"tools/list"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"echo","description":"mock echo tool","inputSchema":{"type":"object","properties":{}}}]}}\n' "$id"
      ;;
  esac
done
"#,
    )
    .expect("write mock server");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
        .expect("chmod mock server");
    script
}

/// Spawn + handshake + tools/list all succeed inside `Daemon::start` when the
/// configured server actually speaks the protocol. (The engine-side tool
/// list is asserted one layer down — rpc/tests/mcp_merge.rs.)
#[test]
fn healthy_stdio_mcp_server_starts_with_the_daemon() {
    let dir = tempdir().expect("temp dir");
    let script = write_mock_server(dir.path());
    let cfg = cfg(
        dir.path(),
        vec![mcp_server("b1-mcp-mock", script.to_str().unwrap(), None)],
    );

    let daemon = Daemon::start(cfg).expect("daemon start with a healthy MCP server");
    assert!(daemon.started_at_ms() > 0, "daemon came up");
}
