//! Discovery + mtime cache (dsh files.ts / state.ts).

use std::fs;

use agent_loop::instruction::{InstructionCache, ancestor_chain, discover};

/// A workspace root: `.git` marker so the upward walk never escapes the
/// tempdir into the real filesystem.
fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    dir
}

/// No AGENTS.md anywhere on the chain: discovery is empty, load is a no-op.
#[test]
fn missing_files_discover_nothing() {
    let root = workspace();
    let nested = root.path().join("a/b");
    fs::create_dir_all(&nested).unwrap();

    assert!(discover(&nested).is_empty());
    assert!(InstructionCache::default().load(&nested).is_empty());
    // The chain itself is bounded by the .git root and outermost-first.
    let chain = ancestor_chain(&nested);
    assert_eq!(chain.first(), Some(&root.path().to_path_buf()));
    assert_eq!(chain.last(), Some(&nested));
}

/// All levels on the chain are found, outermost first.
#[test]
fn finds_every_level_root_outward() {
    let root = workspace();
    fs::write(root.path().join("AGENTS.md"), "root rules").unwrap();
    let sub = root.path().join("a/b");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join("AGENTS.md"), "leaf rules").unwrap();
    fs::write(root.path().join("a/AGENTS.md"), "mid rules").unwrap();

    let found = discover(&sub);
    assert_eq!(
        found,
        vec![
            root.path().join("AGENTS.md"),
            root.path().join("a/AGENTS.md"),
            sub.join("AGENTS.md"),
        ]
    );
    let loaded = InstructionCache::default().load(&sub);
    assert_eq!(loaded.len(), 3);
    assert_eq!(loaded[0].1, "root rules");
    assert_eq!(loaded[2].1, "leaf rules");
}

/// mtime-gated cache: repeated reads are stable, a content change (mtime
/// bump) is picked up on the next read.
#[test]
fn cache_reloads_only_on_mtime_change() {
    let root = workspace();
    let path = root.path().join("AGENTS.md");
    fs::write(&path, "v1").unwrap();

    let mut cache = InstructionCache::default();
    assert_eq!(cache.read(&path).as_deref(), Some("v1"));
    assert_eq!(cache.read(&path).as_deref(), Some("v1")); // cached hit

    fs::write(&path, "v2").unwrap();
    assert_eq!(cache.read(&path).as_deref(), Some("v2"));

    fs::remove_file(&path).unwrap();
    assert_eq!(cache.read(&path), None, "gone file is a no-op, not stale");
}
