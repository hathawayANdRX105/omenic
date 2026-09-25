//! Fragment rendering + digest dedup + the `harness.instruction` service.

use std::fs;
use std::path::PathBuf;

use serde_json::Value;

use agent_loop::instruction::{InstructionCache, InstructionFragments, digest, render_fragments};
use plugin::plugins::InstructionPlugin;
use plugin::{DshPlugin, EventBus, PluginContext, ServiceRegistry};

fn file(path: &str, content: &str) -> (PathBuf, String) {
    (PathBuf::from(path), content.into())
}

/// Two files, different content: both render, outermost first, as
/// `Instructions from:` sections named by path.
#[test]
fn found_files_render_fragments() {
    let frags = render_fragments(&[
        file("/w/AGENTS.md", "root rules"),
        file("/w/a/AGENTS.md", "leaf rules"),
    ]);
    assert_eq!(frags.len(), 2);
    assert_eq!(frags[0].name, "instructions:/w/AGENTS.md");
    assert_eq!(
        frags[0].body,
        "Instructions from: /w/AGENTS.md\n\nroot rules"
    );
    assert!(frags[1].body.contains("leaf rules"));
}

/// Byte-copied / whitespace-padded siblings collapse to one injection
/// (dsh trimmedInstructionDigest duplicate suppression); genuinely
/// different content never collides.
#[test]
fn digest_dedups_identical_content() {
    let frags = render_fragments(&[
        file("/w/AGENTS.md", "same rules\n"),
        file("/w/x/AGENTS.md", "  same rules  "), // trims equal → dropped
        file("/w/x/y/AGENTS.md", "other rules"),
    ]);
    assert_eq!(frags.len(), 2);
    assert_eq!(frags[1].name, "instructions:/w/x/y/AGENTS.md");
    assert_eq!(digest(" a "), digest("a\n\n"));
    assert_ne!(digest("a"), digest("b"));
}

/// No instructions anywhere: the service is provided but empty — hosts
/// injecting fragments observe a no-op, not an absent service.
#[test]
fn plugin_provides_service_empty_when_nothing_found() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    let mut reg = ServiceRegistry::default();
    let mut bus = EventBus::default();
    let ctx = &mut PluginContext::new(&mut reg, &mut bus, &Value::Null);
    InstructionPlugin::new(dir.path()).register(ctx);

    let frags = reg
        .resolve::<InstructionFragments>("harness.instruction")
        .unwrap();
    assert!(frags.0.is_empty());
    assert!(
        reg.resolve::<InstructionFragments>("harness.compaction")
            .is_none()
    );
}

/// Found files: the registered service carries the rendered fragments, and
/// a shared cache keeps repeated refreshes mtime-gated.
#[test]
fn plugin_provides_rendered_fragments() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("AGENTS.md"), "be nice").unwrap();
    let sub = dir.path().join("pkg");
    fs::create_dir(&sub).unwrap();
    fs::write(sub.join("AGENTS.md"), "be nicer").unwrap();

    let mut reg = ServiceRegistry::default();
    let mut bus = EventBus::default();
    let ctx = &mut PluginContext::new(&mut reg, &mut bus, &Value::Null);
    InstructionPlugin::new(&sub).register(ctx);

    let frags = reg
        .resolve::<InstructionFragments>("harness.instruction")
        .unwrap();
    assert_eq!(frags.0.len(), 2);
    assert!(frags.0[0].body.contains("be nice"));

    // Same instance + shared cache: refresh after an edit sees new bytes.
    let plugin = InstructionPlugin::new(&sub);
    let mut cache = InstructionCache::default();
    assert_eq!(plugin.fragments(&mut cache).0.len(), 2);
    fs::write(dir.path().join("AGENTS.md"), "be very nice").unwrap();
    let refreshed = plugin.fragments(&mut cache);
    assert!(refreshed.0[0].body.contains("be very nice"));
}
