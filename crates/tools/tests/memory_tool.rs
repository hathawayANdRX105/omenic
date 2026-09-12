//! Integration tests for the memory tool trio (default-off env gate).
//!
//! Concurrency: `OMENIC_MEMORY*` are process-wide env vars and the tools
//! re-read them on every `execute`, so the tests that flip them serialize
//! on `ENV_LOCK` instead of forcing `--test-threads=1` for the whole
//! binary — the lock keeps the non-env tests parallel and documents the
//! coupling where it lives. Rust 2024 makes `set_var`/`remove_var` unsafe;
//! the only hazard is another thread reading env mid-update, which is
//! exactly what the lock prevents within this binary.

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use serde_json::json;
use tools::Tool;
use tools::memory_tool::{MemoryAppendTool, MemoryListTool, MemorySearchTool};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn sig() -> AtomicBool {
    AtomicBool::new(false)
}

/// Run `body` with the memory env forced to `Some(dir)` (enabled) or
/// `None` (fully unset), restoring the unset state afterwards.
fn with_env<R>(dir: Option<&Path>, body: impl FnOnce() -> R) -> R {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    // SAFETY: every test in this binary that mutates the env holds
    // ENV_LOCK for its whole body, so no other thread reads these vars
    // while they are being rewritten.
    unsafe {
        match dir {
            Some(d) => {
                std::env::set_var("OMENIC_MEMORY", "1");
                std::env::set_var("OMENIC_MEMORY_DIR", d);
            }
            None => {
                std::env::remove_var("OMENIC_MEMORY");
                std::env::remove_var("OMENIC_MEMORY_DIR");
            }
        }
    }
    let out = body();
    unsafe {
        std::env::remove_var("OMENIC_MEMORY");
        std::env::remove_var("OMENIC_MEMORY_DIR");
    }
    out
}

#[test]
fn default_off_returns_readable_hint_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("mem");
    let s = sig();

    // Env entirely unset: all three tools say "disabled", no I/O.
    with_env(None, || {
        for out in [
            MemoryAppendTool.execute(&json!({"text": "x"}), &s).unwrap(),
            MemorySearchTool
                .execute(&json!({"query": "x"}), &s)
                .unwrap(),
            MemoryListTool.execute(&json!({}), &s).unwrap(),
        ] {
            assert!(out.starts_with("memory disabled"), "got: {out}");
        }
        assert!(!store.exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    });

    // OMENIC_MEMORY=1 but the dir does not exist: still disabled, and the
    // missing directory must NOT be created by the tools.
    with_env(Some(&store), || {
        let out = MemoryAppendTool.execute(&json!({"text": "x"}), &s).unwrap();
        assert!(out.starts_with("memory disabled"), "got: {out}");
        assert!(
            !store.exists(),
            "a disabled handle must never create the store dir"
        );
    });
}

#[test]
fn enabled_append_then_search_and_list_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let s = sig();

    with_env(Some(dir.path()), || {
        let first = MemoryAppendTool
            .execute(
                &json!({"text": "ketchup belongs on eggs", "category": "preference", "tags": ["food"]}),
                &s,
            )
            .unwrap();
        assert_eq!(first, "remembered #1");
        let second = MemoryAppendTool
            .execute(&json!({"text": "the repo builds with cargo test"}), &s)
            .unwrap();
        assert_eq!(second, "remembered #2");

        // The store is the single source of truth on disk.
        assert!(dir.path().join("memory.jsonl").is_file());

        // Search: one line per hit, id + score + text. Only #1 matches;
        // #2 shares no tokens and no tags, so nothing cascades in.
        let hits = MemorySearchTool
            .execute(&json!({"query": "ketchup eggs"}), &s)
            .unwrap();
        let lines: Vec<&str> = hits.lines().collect();
        assert_eq!(lines.len(), 2, "got: {hits}"); // header + 1 hit
        assert_eq!(lines[0], "1 hits:");
        assert!(lines[1].starts_with("#1 score="), "got: {}", lines[1]);
        assert!(lines[1].contains("ketchup belongs on eggs"));

        // Misses get an explicit hint, not an empty string.
        let miss = MemorySearchTool
            .execute(&json!({"query": "pineapple"}), &s)
            .unwrap();
        assert!(miss.starts_with("no memory hits"), "got: {miss}");

        // List: `#<id> [<category>] <text>`, one line per active entry.
        let all = MemoryListTool.execute(&json!({}), &s).unwrap();
        let lines: Vec<&str> = all.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "#1 [preference] ketchup belongs on eggs");
        assert_eq!(lines[1], "#2 [custom] the repo builds with cargo test");
    });
}

#[test]
fn list_shows_most_recent_50_and_skips_superseded() {
    let dir = tempfile::tempdir().unwrap();
    let s = sig();

    with_env(Some(dir.path()), || {
        for i in 1..=55u64 {
            let out = MemoryAppendTool
                .execute(&json!({"text": format!("note {i}")}), &s)
                .unwrap();
            assert_eq!(out, format!("remembered #{i}"));
        }

        let all = MemoryListTool.execute(&json!({}), &s).unwrap();
        let lines: Vec<&str> = all.lines().collect();
        assert_eq!(lines.len(), 50, "list must cap at the most recent 50");
        assert_eq!(lines[0], "#6 [custom] note 6");
        assert_eq!(lines[49], "#55 [custom] note 55");

        // Retire #10 through the store API (no tool for it yet): supersede
        // marks the old row inactive, so it drops out of the listing.
        let mut mem = memory::Memory::open(dir.path()).unwrap();
        mem.supersede(10, memory::MemoryEntry::new("note 10 corrected"))
            .unwrap();
        let all = MemoryListTool.execute(&json!({}), &s).unwrap();
        let lines: Vec<&str> = all.lines().collect();
        assert_eq!(lines.len(), 50, "window shifts: #56 in, #5 out");
        assert_eq!(lines[0], "#6 [custom] note 6");
        assert_eq!(lines[49], "#56 [custom] note 10 corrected");
        assert!(!all.contains("#10 "), "superseded entry must not show");
    });
}

#[test]
fn argument_validation_runs_before_the_env_gate() {
    let s = sig();
    // No ENV_LOCK needed: every case fails during arg parsing, before
    // memory_store() is consulted, so the env state cannot matter.

    let err = MemoryAppendTool
        .execute(&json!({}), &s)
        .unwrap_err()
        .to_string();
    assert_eq!(err, "missing string argument: text");

    let err = MemoryAppendTool
        .execute(&json!({"text": "x", "category": "vibes"}), &s)
        .unwrap_err()
        .to_string();
    assert!(err.starts_with("invalid category: vibes"), "got: {err}");

    let err = MemoryAppendTool
        .execute(&json!({"text": "x", "tags": "food"}), &s)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("tags must be an array of strings"),
        "got: {err}"
    );

    let err = MemorySearchTool
        .execute(&json!({}), &s)
        .unwrap_err()
        .to_string();
    assert_eq!(err, "missing string argument: query");
}

#[test]
fn trio_is_registered_with_sane_schemas() {
    let names: Vec<String> = tools::builtin_tools()
        .iter()
        .map(|t| t.name().to_string())
        .collect();
    for name in ["memory_append", "memory_search", "memory_list"] {
        assert!(names.iter().any(|n| n == name), "{name} not registered");
    }

    let schema = MemoryAppendTool.parameters();
    assert_eq!(
        schema["properties"]["category"]["enum"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert_eq!(schema["required"][0], "text");
    assert_eq!(MemorySearchTool.parameters()["required"][0], "query");
    assert_eq!(
        MemoryListTool.parameters()["properties"]
            .as_object()
            .unwrap()
            .len(),
        0
    );
}
