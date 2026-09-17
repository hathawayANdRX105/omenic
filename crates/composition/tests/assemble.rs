//! Composition root tests (C6.5): the assembled container is real.
//!
//! Before G5 `assemble()` had no caller and no test, so nothing proved the
//! registration order or the duplicate-name guard. These tests pin both, plus
//! the config-driven knobs, so a later host plugin cannot silently shadow a
//! core service.
//!
//! Hermetic instruction discovery: `InstructionPlugin` walks the directory
//! chain reading `AGENTS.md`, so every test points `cwd` at a fresh empty
//! temp dir instead of the repository (which does have an `AGENTS.md` and
//! would make assertions depend on its contents).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use tempfile::TempDir;

use omenic_composition::{DshPlugin, PluginError, assemble};
use omenic_harness_compaction::CharBudgetPolicy;
use omenic_harness_instruction::InstructionFragments;
use omenic_harness_plugin::PluginContext;
use omenic_harness_prompt::PromptTemplate;
use omenic_harness_runtime::LoopEngine;
use omenic_harness_tools::ToolCatalog;

/// Empty directory with no `AGENTS.md` anywhere it would be discovered.
///
/// `InstructionPlugin` walks the directory chain reading `AGENTS.md`, so every
/// test points `cwd` here instead of at the repository (which has one, and
/// would make assertions depend on its contents). Backed by [`TempDir`]:
/// uniqueness and cleanup are handled by the same helper the rest of the
/// workspace's tests use.
struct EmptyCwd {
    // Held only so the directory is removed on drop; never read directly.
    #[allow(dead_code)] // rationale: lifetime anchor for the temp dir
    dir: TempDir,
    path: String,
}

impl EmptyCwd {
    fn new(_tag: &str) -> EmptyCwd {
        let dir = TempDir::new().expect("create temp cwd");
        // Resolve once: the path is consumed by `json!` in every test, and a
        // temp path is UTF-8 on every supported platform, so paying for the
        // check once beats re-validating (and risking a panic) per call.
        let path = dir.path().to_str().expect("temp path is utf-8").to_string();
        EmptyCwd { dir, path }
    }

    fn as_str(&self) -> &str {
        &self.path
    }
}

/// A host plugin that records whether it ran, under a caller-chosen name.
struct SpyPlugin {
    name: String,
    ran: Arc<AtomicUsize>,
}

impl DshPlugin for SpyPlugin {
    fn name(&self) -> &str {
        &self.name
    }

    fn register(&self, _ctx: &mut PluginContext<'_>) {
        self.ran.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn core_plugins_register_in_dependency_order() {
    let cwd = EmptyCwd::new("order");
    let (_fiber, registry) = assemble(json!({ "cwd": cwd.as_str() }), Vec::new())
        .expect("assemble with no host plugins");

    // Compaction before instruction: the order the assembly root documents,
    // and the order a host must be able to rely on when it re-provides one.
    assert_eq!(
        registry.plugins(),
        vec!["harness-compaction", "harness-instruction"]
    );
}

#[test]
fn every_harness_service_resolves_from_the_assembled_fiber() {
    let cwd = EmptyCwd::new("services");
    let (mut fiber, _registry) = assemble(
        json!({ "cwd": cwd.as_str(), "system_prompt": "BASE", "model": "m1", "max_turns": 7 }),
        Vec::new(),
    )
    .expect("assemble");

    let ctx = fiber.context();
    // Provided directly by the root, in registration order.
    let prompt = ctx
        .resolve::<PromptTemplate>("harness.prompt")
        .expect("harness.prompt");
    assert_eq!(prompt.body, "BASE");
    assert!(ctx.resolve::<ToolCatalog>("harness.tools").is_some());
    let engine = ctx
        .resolve::<LoopEngine>("harness.loop")
        .expect("harness.loop");
    assert_eq!(engine.max_turns, 7);
    assert_eq!(engine.model, "m1");
    // Provided by the two core plugins — the point of G5's wiring.
    assert!(
        ctx.resolve::<CharBudgetPolicy>("harness.compaction")
            .is_some(),
        "CompactionPlugin must provide harness.compaction"
    );
    assert!(
        ctx.resolve::<InstructionFragments>("harness.instruction")
            .is_some(),
        "InstructionPlugin must provide harness.instruction"
    );
}

#[test]
fn empty_config_falls_back_to_neutral_loop_defaults() {
    let cwd = EmptyCwd::new("defaults");
    let (mut fiber, _registry) =
        assemble(json!({ "cwd": cwd.as_str() }), Vec::new()).expect("assemble");

    let ctx = fiber.context();
    let engine = ctx
        .resolve::<LoopEngine>("harness.loop")
        .expect("harness.loop");
    assert_eq!(engine.max_turns, 64, "documented default");
    assert_eq!(engine.model, "default");
    let prompt = ctx
        .resolve::<PromptTemplate>("harness.prompt")
        .expect("harness.prompt");
    assert_eq!(prompt.body, "", "absent system_prompt stays empty");
}

#[test]
fn host_plugin_reusing_a_core_name_is_rejected() {
    let cwd = EmptyCwd::new("dup");
    let ran = Arc::new(AtomicUsize::new(0));
    let clash = Arc::new(SpyPlugin {
        name: "harness-compaction".into(),
        ran: Arc::clone(&ran),
    });

    // `assemble` returns a `(Fiber, PluginRegistry)` on success, neither of
    // which is `Debug`, so neither `expect_err` nor `unwrap_err` can be used
    // (both need to print the Ok value on failure) — match exhaustively.
    match assemble(json!({ "cwd": cwd.as_str() }), vec![clash]) {
        Ok(_) => panic!("a duplicate core plugin name must abort assembly"),
        // Exhaustive over the current `PluginError`: a future variant makes
        // this match non-exhaustive by design, so the test is forced to
        // decide the new case instead of silently passing.
        Err(PluginError::Duplicate(name)) => assert_eq!(name, "harness-compaction"),
        Err(PluginError::InvalidConfig(msg)) => panic!("unexpected config error: {msg}"),
    }
    // The registry rejects before running the plugin, so the shadowing host
    // plugin never got to touch the context.
    assert_eq!(
        ran.load(Ordering::SeqCst),
        0,
        "rejected plugin must not have registered"
    );
}

#[test]
fn host_plugins_register_after_the_core_ones() {
    let cwd = EmptyCwd::new("host");
    let ran = Arc::new(AtomicUsize::new(0));
    let host = Arc::new(SpyPlugin {
        name: "host-extra".into(),
        ran: Arc::clone(&ran),
    });

    let (_fiber, registry) =
        assemble(json!({ "cwd": cwd.as_str() }), vec![host]).expect("assemble with a host plugin");

    assert_eq!(
        registry.plugins(),
        vec!["harness-compaction", "harness-instruction", "host-extra"],
        "host plugins go last so they may provide over core services"
    );
    assert_eq!(ran.load(Ordering::SeqCst), 1);
}
