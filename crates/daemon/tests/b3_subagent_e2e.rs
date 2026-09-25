//! B3 smoke — the ACP subagent path behind the real daemon.
//!
//! `acp_provider_test.rs` proves `AcpProvider` drives a child over stdio. It
//! does not prove the daemon wires it up: that needs `Daemon::start` ->
//! `orbit_setup` (config -> `AcpProviderSpec` -> registry) -> catalog ->
//! `combined_tools` -> the loop's tool dispatch, all real, with the request
//! leaving the process.
//!
//! These tests close that gap with the crate's own `mock_acp_server` binary
//! as the child. The mock lives in the `omenic-harness-subagent` crate; both
//! it and this test binary land in the same target dir under the same
//! profile, so the sibling-next-to-me lookup is stable.

use daemon::{Daemon, DaemonClient};
use serde_json::{Value, json};
use tempfile::tempdir;

use common::{MockOpenAi, daemon_cfg, drain_events, one_text_turn, prompt};

mod common;

/// One assistant round that issues a single tool call and finishes the turn.
/// Duplicated from `b3_tools_e2e.rs` on purpose: each integration test file
/// compiles as its own crate, and a shared `mod` would mean one helper set
/// per binary with dead-code lint noise in the others.
fn tool_turn(name: &str, args: &Value) -> String {
    let call = json!({
        "choices": [{
            "delta": {
                "role": "assistant",
                "tool_calls": [{
                    "index": 0,
                    "id": "call_b3_sub",
                    "type": "function",
                    "function": { "name": name, "arguments": args.to_string() }
                }]
            },
            "finish_reason": Value::Null
        }]
    });
    let finish = json!({
        "choices": [{ "delta": {}, "finish_reason": "tool_calls" }]
    });
    format!("data: {}\ndata: {}\n\n", call, finish)
}

/// Path to the mock ACP agent binary.
///
/// `CARGO_BIN_EXE_mock_acp_server` only resolves inside the
/// `omenic-harness-subagent` crate's own tests; from here the binary is found
/// relative to this test executable — cargo test runs it from
/// `target/<profile>/deps`, so the parent of that is `target/<profile>`,
/// where the mock lands as a sibling build product.
fn mock_acp_server() -> String {
    std::env::current_exe()
        .expect("test executable path")
        .parent()
        .and_then(|deps| deps.parent())
        .expect("target dir above the deps dir")
        .join("mock_acp_server")
        .to_string_lossy()
        .into_owned()
}

/// A daemon config with one out-of-process provider named `mock` running the
/// crate's scripted ACP agent under `env`.
fn cfg_with_mock_provider(
    dir: &std::path::Path,
    mock: &MockOpenAi,
    env: &[(&str, &str)],
) -> daemon::DaemonConfig {
    let mut cfg = daemon_cfg(dir, mock, 4);
    cfg.subagent_providers = vec![config::SubagentProviderConfig {
        name: "mock".into(),
        command: mock_acp_server(),
        args: Vec::new(),
        env: env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        cwd: None,
        permission: config::SubagentPermission::Allow,
        dispose_grace_ms: None,
        dispose_eof_grace_ms: None,
    }];
    cfg
}

/// Run a daemon on `dir` against `replies` and return every HTTP body the
/// mock saw, in arrival order — body *n+1* carries round *n*'s tool results.
fn run_turns(dir: &std::path::Path, cfg: daemon::DaemonConfig) {
    let _ = std::fs::write(dir.join("AGENTS.md"), "# smoke\n");
    let mut daemon = Daemon::start(cfg).expect("daemon start");

    let client = DaemonClient::connect_to(daemon.socket_addr().path());
    let mut sub = client.subscribe("worker").expect("subscribe");
    prompt(&client, "go");
    drain_events(&mut sub);
    drop(client);
    daemon.shutdown();
}

/// The `subagent` tool through the real daemon: the ACP child's streamed text
/// must reach the next request body the loop sends — only a working
/// config->orbit_setup->registry->catalog->dispatch chain can put it there.
#[test]
fn acp_provider_round_trip_through_daemon() {
    let dir = tempdir().expect("tempdir");
    let marker = "hello from the acp backend";
    let mock = MockOpenAi::start(vec![
        tool_turn("subagent", &json!({"provider": "mock", "prompt": "say it"})),
        one_text_turn("done"),
    ]);
    let cfg = cfg_with_mock_provider(dir.path(), &mock, &[("MOCK_TEXT", marker)]);
    let bodies = {
        run_turns(dir.path(), cfg);
        mock.received()
    };

    assert!(bodies.len() >= 2, "loop must post at least two requests");
    assert!(
        bodies[1].contains(marker),
        "the ACP child's text must come back in the next request, got: {}",
        bodies[1]
    );
}

/// `subagent_control list` through the real daemon: both the configured
/// out-of-process provider and the built-in `fork` must be named, proving
/// `orbit_setup` registered both.
#[test]
fn control_list_names_configured_and_builtin_providers() {
    let dir = tempdir().expect("tempdir");
    let mock = MockOpenAi::start(vec![
        tool_turn("subagent_control", &json!({"action": "list"})),
        one_text_turn("done"),
    ]);
    let cfg = cfg_with_mock_provider(dir.path(), &mock, &[]);
    let bodies = {
        run_turns(dir.path(), cfg);
        mock.received()
    };

    assert!(bodies.len() >= 2, "loop must post at least two requests");
    assert!(
        bodies[1].contains("mock") && bodies[1].contains("fork"),
        "list must name the configured provider and the built-in one, got: {}",
        bodies[1]
    );
}

/// A provider the config never named must fail as a tool error without
/// breaking the daemon: the following turn still dispatches. A bad command
/// (not an unknown name) is covered by `acp_provider_test.rs`.
#[test]
fn unknown_provider_fails_without_breaking_daemon() {
    let dir = tempdir().expect("tempdir");
    let mock = MockOpenAi::start(vec![
        tool_turn("subagent", &json!({"provider": "ghost", "prompt": "x"})),
        tool_turn("subagent_control", &json!({"action": "list"})),
        one_text_turn("done"),
    ]);
    let cfg = cfg_with_mock_provider(dir.path(), &mock, &[]);
    let bodies = {
        run_turns(dir.path(), cfg);
        mock.received()
    };

    assert!(bodies.len() >= 3, "the daemon must keep serving turns");
    assert!(
        bodies[1].contains("ghost"),
        "the unknown-provider error must reach the model, got: {}",
        bodies[1]
    );
    assert!(
        bodies[2].contains("fork"),
        "a later turn must still dispatch after a tool error, got: {}",
        bodies[2]
    );
}
