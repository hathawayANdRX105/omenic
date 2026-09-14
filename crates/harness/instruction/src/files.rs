//! AGENTS.md discovery up the directory chain + mtime-gated loading.
//!
//! Reference: dsh `agent-instructions/src/files.ts`
//! (`findProjectRoot` + `ancestorChain` + candidate loop) and `state.ts`
//! (reload only when a file's mtime moved).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Same-directory instruction candidates, ordered. dsh also ships
/// `CLAUDE.md` + `*.local.md` overlays; omenic only needs the one voice.
pub const INSTRUCTION_CANDIDATES: &[&str] = &["AGENTS.md"];

/// Directory entry whose presence marks a project root and stops the
/// upward walk (dsh `projectRootMarkers`, first marker only).
pub const ROOT_MARKER: &str = ".git";

/// Ancestor directories, outermost (broadest) first, cwd last — the order
/// instructions render in, so more specific files read later and win
/// (dsh `ancestorChain` semantics; walk bounded by [`ROOT_MARKER`] or the
/// filesystem root).
pub fn ancestor_chain(cwd: &Path) -> Vec<PathBuf> {
    let mut chain = Vec::new();
    let mut dir = Some(cwd.to_path_buf());
    while let Some(d) = dir {
        let at_root = d.join(ROOT_MARKER).exists();
        let next = d.parent().map(Path::to_path_buf);
        chain.push(d);
        if at_root || next.is_none() {
            break;
        }
        dir = next;
    }
    chain.reverse();
    chain
}

/// Every existing instruction file on the chain, outermost first, deduped
/// by path (dsh `discoverInstructionFiles` minus the user-global home dir).
pub fn discover(cwd: &Path) -> Vec<PathBuf> {
    let mut seen: Vec<PathBuf> = Vec::new();
    for dir in ancestor_chain(cwd) {
        for name in INSTRUCTION_CANDIDATES {
            let path = dir.join(name);
            if path.is_file() && !seen.contains(&path) {
                seen.push(path);
            }
        }
    }
    seen
}

/// mtime-gated read cache (dsh `state.ts`): bytes are re-read from disk
/// only when a file's mtime moved.
#[derive(Default)]
pub struct InstructionCache {
    entries: HashMap<PathBuf, (SystemTime, String)>,
}

impl InstructionCache {
    /// Read one instruction file, serving the cached copy while its mtime
    /// is unchanged. `None` when the file is gone or unreadable — discovery
    /// then rendering treat a missing file as a no-op.
    pub fn read(&mut self, path: &Path) -> Option<String> {
        let mtime = fs::metadata(path).ok()?.modified().ok()?;
        if let Some((t, content)) = self.entries.get(path) {
            if *t == mtime {
                return Some(content.clone());
            }
        }
        let content = fs::read_to_string(path).ok()?;
        self.entries
            .insert(path.to_path_buf(), (mtime, content.clone()));
        Some(content)
    }

    /// Discover under `cwd` and load every hit, outermost first.
    pub fn load(&mut self, cwd: &Path) -> Vec<(PathBuf, String)> {
        discover(cwd)
            .into_iter()
            .filter_map(|p| self.read(&p).map(|c| (p, c)))
            .collect()
    }
}
