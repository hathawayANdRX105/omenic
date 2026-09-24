//! The instruction plugin: provides rendered workspace fragments as the
//! `harness.instruction` service.

use std::path::PathBuf;

use plugin::{DshPlugin, PluginContext};

use crate::files::InstructionCache;
use crate::render::{InstructionFragments, render_fragments};

/// Service key of the rendered fragments (matches the plugin name domain).
pub const INSTRUCTION_SERVICE: &str = "harness.instruction";

/// Discovers, loads and renders instructions under `cwd` once at register
/// time, then provides the result. Hosts refreshing mid-session call
/// [`InstructionPlugin::fragments`] with a long-lived
/// [`InstructionCache`] so unchanged files are never re-read.
pub struct InstructionPlugin {
    cwd: PathBuf,
}

impl Default for InstructionPlugin {
    fn default() -> Self {
        Self::new(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }
}

impl InstructionPlugin {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        InstructionPlugin { cwd: cwd.into() }
    }

    /// Load (mtime-cached) + render (digest-deduped) right now.
    pub fn fragments(&self, cache: &mut InstructionCache) -> InstructionFragments {
        InstructionFragments(render_fragments(&cache.load(&self.cwd)))
    }
}

impl DshPlugin for InstructionPlugin {
    fn name(&self) -> &str {
        "harness-instruction"
    }

    fn register(&self, ctx: &mut PluginContext<'_>) {
        ctx.provide(
            INSTRUCTION_SERVICE,
            self.fragments(&mut InstructionCache::default()),
        );
    }
}
