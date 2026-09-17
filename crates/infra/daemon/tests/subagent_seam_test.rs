//! Subagent seam: the daemon registers an in-process fork provider into the
//! container's `SubagentRuntimeService` (fiber key `harness.subagents`), so the
//! model-facing `subagent` tool resolves a provider in the daemon.
//!
//! These tests reproduce the registration block from `server.rs` (same
//! service type, same key, same `ForkProvider` construction) plus the
//! read-only tool filter, and assert:
//! - register → get returns `Some` and the provider identity round-trips
//!   (`name()` is `"fork"`);
//! - registering a second provider under the same name overwrites (the
//!   service's documented contract — composition controls load order);
//! - the read-only tool filter via `filter_builtin_tools` yields exactly
//!   the three fork tools.

use std::sync::Arc;

use orbit::LlmBackend;

use omenic_harness_subagent::{ForkProvider, SubagentRuntimeService};

/// The daemon's fork provider registration (server.rs block, condensed):
/// a service + a `ForkProvider` built over the read-only tool subset.
fn register_fork(service: &SubagentRuntimeService) {
    let model = adaptor::Model {
        api_key: "test".into(),
        model: "test-model".into(),
        base_url: None,
        max_tokens: None,
    };
    let backend: Arc<dyn LlmBackend + Send + Sync> = Arc::new(orbit::HttpLlm);
    let wanted = ["read_file", "grep", "glob"];
    let tools: Arc<Vec<Box<dyn tools::Tool>>> =
        Arc::new(omenic_harness_tools::filter_builtin_tools(&wanted));
    service.register(
        "fork",
        Arc::new(ForkProvider::new(backend, model, tools, 8)),
    );
}

/// After `register("fork", …)` the service hands back a provider that
/// identifies as `"fork"` — the seam is live, not a throwaway.
#[test]
fn fork_provider_registers_and_resolves() {
    let service = SubagentRuntimeService::default();
    assert!(
        service.get("fork").is_none(),
        "no provider before registration"
    );
    register_fork(&service);
    let provider = service.get("fork").expect("fork provider must resolve");
    assert_eq!(provider.name(), "fork");
}

/// Re-registering under the same name overwrites (idempotent-safe; the
/// documented composition-order contract — last writer wins).
#[test]
fn re_register_same_name_overwrites() {
    let service = SubagentRuntimeService::default();
    register_fork(&service);
    register_fork(&service);
    let provider = service.get("fork").expect("fork provider must resolve");
    assert_eq!(provider.name(), "fork");
    assert_eq!(service.providers(), vec!["fork".to_string()]);
}

/// The daemon's filter over `tools::builtin_tools()` yields exactly the
/// three read-only tool names — no write/exec tool leaks into a fork run.
#[test]
fn fork_tools_filter_yields_read_only_subset() {
    let wanted = ["read_file", "grep", "glob"];
    let names: Vec<String> = omenic_harness_tools::filter_builtin_tools(&wanted)
        .into_iter()
        .map(|t| tools::def(&*t).name)
        .collect();
    assert_eq!(
        names,
        vec![
            "read_file".to_string(),
            "grep".to_string(),
            "glob".to_string()
        ],
        "fork tool set is exactly the read-only built-in subset"
    );
}
