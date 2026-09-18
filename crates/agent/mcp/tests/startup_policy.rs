//! Startup-policy tests: per-server `cwd` handling in
//! [`mcp::StdioTransport::spawn`] and `fail_on_startup_error` in
//! [`mcp::external_tools_from_mcp`].
//!
//! No real MCP server needed. A nonexistent command fails at spawn
//! (`McpError::Spawn`); a real but non-MCP binary (`pwd`) starts fine and
//! fails the handshake with a transport error instead — exactly the
//! distinction the cwd wiring must preserve.

use std::sync::atomic::AtomicBool;

use config::McpServerConfig;
use mcp::{Mcp, McpError, external_tools_from_mcp};

/// A minimal stdio server config; fields beyond `name`/`command` stay at
/// their defaults.
fn server(name: &str, command: &str) -> McpServerConfig {
    McpServerConfig {
        name: name.to_string(),
        command: Some(command.to_string()),
        args: Vec::new(),
        env: std::collections::HashMap::new(),
        url: None,
        tool_call_timeout_ms: None,
        reconnect: None,
        cwd: None,
        fail_on_startup_error: None,
    }
}

/// `expect_err` without a `Debug` bound on the success type: neither `Mcp`
/// nor `Vec<Box<dyn Tool>>` implements `Debug`, so the stdlib helper is
/// unusable on these results.
fn unwrap_err<T>(res: Result<T, McpError>, msg: &str) -> McpError {
    match res {
        Ok(_) => panic!("{msg}"),
        Err(e) => e,
    }
}

const GHOST: &str = "definitely-not-a-real-binary-omenic";

#[test]
fn absent_flag_skips_failing_server() {
    // Default policy: a server whose command does not exist contributes zero
    // tools and is skipped; the bring-up still succeeds.
    let signal = AtomicBool::new(false);
    let cfg = server("ghost", GHOST);
    let tools = external_tools_from_mcp(&[cfg], &signal).expect("absent flag keeps Ok");
    assert!(tools.is_empty());
}

#[test]
fn fail_flag_propagates_spawn_error() {
    let signal = AtomicBool::new(false);
    let mut cfg = server("ghost", GHOST);
    cfg.fail_on_startup_error = Some(true);
    let err = unwrap_err(
        external_tools_from_mcp(&[cfg], &signal),
        "fail flag must error",
    );
    assert!(matches!(err, McpError::Spawn(_)), "got {err:?}");
}

#[test]
fn fail_flag_aborts_before_later_servers() {
    // The failing server comes first, so a later entry with the default
    // (skip) policy is never reached: the whole bring-up aborts at the first
    // error instead of returning Ok with the survivors.
    let signal = AtomicBool::new(false);
    let mut bad = server("ghost", GHOST);
    bad.fail_on_startup_error = Some(true);
    let later = server("later-ghost", "also-not-a-real-binary-omenic");
    let err = unwrap_err(
        external_tools_from_mcp(&[bad, later], &signal),
        "first failure aborts",
    );
    assert!(matches!(err, McpError::Spawn(_)), "got {err:?}");
}

#[test]
fn valid_cwd_spawns_but_handshake_fails_non_spawn() {
    // `pwd` starts fine with cwd /tmp (the directory is accepted) but is not
    // an MCP server, so the handshake fails with a non-Spawn error. Proves
    // the cwd wiring does not break process creation.
    let signal = AtomicBool::new(false);
    let mut cfg = server("pwd", "pwd");
    cfg.cwd = Some("/tmp".into());
    let err = unwrap_err(Mcp::spawn(&cfg, &signal), "pwd is not an MCP server");
    assert!(!matches!(err, McpError::Spawn(_)), "got {err:?}");
}

#[test]
fn empty_cwd_is_ignored() {
    // An empty cwd means inherit, same as an absent field — applying it
    // would make `current_dir("")` fail the spawn.
    let signal = AtomicBool::new(false);
    let mut cfg = server("pwd", "pwd");
    cfg.cwd = Some(String::new());
    let err = unwrap_err(Mcp::spawn(&cfg, &signal), "pwd is not an MCP server");
    assert!(!matches!(err, McpError::Spawn(_)), "got {err:?}");
}

#[test]
fn bad_cwd_fails_spawn_with_path_named() {
    // A nonexistent cwd makes `Command::spawn` fail; the error must be
    // `McpError::Spawn` and must name the directory so a missing cwd is
    // distinguishable from a missing program.
    let signal = AtomicBool::new(false);
    let mut cfg = server("pwd", "pwd");
    cfg.cwd = Some("/definitely/not/a/dir-omenic".into());
    let err = unwrap_err(Mcp::spawn(&cfg, &signal), "bad cwd must fail spawn");
    match err {
        McpError::Spawn(m) => {
            assert!(
                m.contains("/definitely/not/a/dir-omenic"),
                "message must name the cwd: {m}"
            );
        }
        other => panic!("expected Spawn error, got {other:?}"),
    }
}
