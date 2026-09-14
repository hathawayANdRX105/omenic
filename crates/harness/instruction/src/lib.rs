//! Workspace instruction injection (C4 instruction side).
//!
//! Discovers `AGENTS.md` files up the directory chain (cwd outward to the
//! project root), loads them through an mtime-gated cache, and renders them
//! into deduplicated [`PromptTemplate`] fragments for the host to inject.
//!
//! Reference: dsh `packages/context/agent-instructions` — `files.ts`
//! (`discoverInstructionFiles` / `ancestorChain` / candidate names),
//! `state.ts` (mtime-gated content cache), `render.ts` (`Instructions
//! from:` sections, more-specific-wins ordering), `digest.ts`
//! (`trimmedInstructionDigest` duplicate suppression).

pub mod files;
pub mod plugin;
pub mod render;

pub use files::{INSTRUCTION_CANDIDATES, InstructionCache, ancestor_chain, discover};
pub use plugin::{INSTRUCTION_SERVICE, InstructionPlugin};
pub use render::{InstructionFragments, digest, render_fragments};
