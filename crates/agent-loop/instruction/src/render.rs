//! Instruction fragments → prompt templates, with digest dedup.
//!
//! Reference: dsh `agent-instructions/src/render.ts` (`sectionText`:
//! `Instructions from: <path>` + body) and `digest.ts`
//! (`trimmedInstructionDigest`: same-content files collapse to one
//! rendered section).

use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use prompt::PromptTemplate;

/// Whitespace-insensitive content identity (dsh hashes trimmed content with
/// SHA-1 because digests are persisted in session state; ours live only
/// inside one process, so std SipHash is enough — swap to a sha1 crate the
/// day a digest leaves the process).
pub fn digest(content: &str) -> u64 {
    let mut h = DefaultHasher::new();
    content.trim().hash(&mut h);
    h.finish()
}

/// The rendered fragments, provided as the `harness.instruction` service.
#[derive(Debug, Clone, Default)]
pub struct InstructionFragments(pub Vec<PromptTemplate>);

/// Render loaded instruction files into injectable prompt fragments.
///
/// One fragment per file (outermost first), each named after its source
/// path; a file whose trimmed digest was already injected is skipped —
/// byte-copied or symlinked siblings collapse to a single section (dsh
/// duplicate suppression).
pub fn render_fragments(files: &[(PathBuf, String)]) -> Vec<PromptTemplate> {
    let mut seen = HashSet::new();
    files
        .iter()
        .filter(|(_, content)| seen.insert(digest(content)))
        .map(|(path, content)| {
            PromptTemplate::new(
                format!("instructions:{}", path.display()),
                format!("Instructions from: {}\n\n{content}", path.display()),
            )
        })
        .collect()
}
