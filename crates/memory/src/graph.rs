//! Derived memory graph: entries are the source of truth (JSONL), the graph
//! is a rebuildable view over them.
//!
//! Data model ported from jcode's `jcode-memory-types` (HashMap adjacency,
//! no petgraph — JSON-friendly and good enough at local-store scale). Edge
//! kinds are pruned to what omenic can actually derive today:
//! `SharedTag` / `Supersedes` / `Contradicts`. jcode's own plan judged
//! tag-nodes and clusters "write-heavy, read-never"; `RelatesTo` and
//! `DerivedFrom` arrive with the explicit `link` action and the extraction
//! pipeline (MEM-3/MEM-4).

use std::collections::HashMap;

use crate::MemoryEntry;

/// Directed edge between two real entries, derived from entry fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgeKind {
    /// Same user tag.
    SharedTag(String),
    /// This entry replaces `superseded_by`'s target.
    Supersedes,
    /// This entry contradicts the target (both stay active for triage).
    Contradicts,
}

impl EdgeKind {
    /// BFS traversal weight (jcode edge weights: HasTag 0.8 / Supersedes 0.9 /
    /// Contradicts 0.3 — contradictions are noise, not signal).
    pub fn traversal_weight(&self) -> f32 {
        match self {
            EdgeKind::SharedTag(_) => 0.8,
            EdgeKind::Supersedes => 0.9,
            EdgeKind::Contradicts => 0.3,
        }
    }
}

/// One directed edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub to: u64,
    pub kind: EdgeKind,
}

/// Adjacency view over the entry store. Rebuildable by deleting it.
///
/// `forward` holds the edges as derived (old → replacement, entry →
/// contradiction target, tag match → peer); `reverse` holds the same edges
/// mirrored with their kind, so walks in either direction see weights
/// without re-scanning.
#[derive(Debug, Default, Clone)]
pub struct MemoryGraph {
    pub memories: HashMap<u64, MemoryEntry>,
    pub forward: HashMap<u64, Vec<Edge>>,
    pub reverse: HashMap<u64, Vec<Edge>>,
}

impl MemoryGraph {
    /// Build the graph from id-sorted entries. Latest-wins is already applied
    /// by [`crate::Memory::list`]; entries superseded by a newer one keep
    /// their edges so the history stays traversable.
    pub fn build(entries: &[MemoryEntry]) -> MemoryGraph {
        let mut graph = MemoryGraph {
            memories: entries.iter().map(|e| (e.id, e.clone())).collect(),
            forward: HashMap::new(),
            reverse: HashMap::new(),
        };
        for e in entries {
            graph.forward.entry(e.id).or_default();
        }
        // Tag → entries index, built once: the pairwise tag match would be
        // O(n²) over the store otherwise.
        let mut tag_index: HashMap<&str, Vec<u64>> = HashMap::new();
        for e in entries {
            for tag in &e.tags {
                tag_index.entry(tag.as_str()).or_default().push(e.id);
            }
        }
        for e in entries {
            let mut matches: Vec<(String, u64)> = Vec::new();
            for tag in &e.tags {
                for other in tag_index
                    .get(tag.as_str())
                    .map(|v| v.as_slice())
                    .unwrap_or(&[])
                {
                    if *other != e.id {
                        matches.push((tag.clone(), *other));
                    }
                }
            }
            // Deterministic edge order regardless of index iteration order.
            matches.sort();
            matches.dedup();
            for (tag, to) in matches {
                graph.link(e.id, to, EdgeKind::SharedTag(tag));
            }
            if let Some(newer) = e.superseded_by {
                graph.link(e.id, newer, EdgeKind::Supersedes);
            }
            if let Some(conflict) = e.contradicts {
                graph.link(e.id, conflict, EdgeKind::Contradicts);
            }
        }
        graph
    }

    fn link(&mut self, from: u64, to: u64, kind: EdgeKind) {
        self.forward.entry(from).or_default().push(Edge {
            to,
            kind: kind.clone(),
        });
        self.reverse
            .entry(to)
            .or_default()
            .push(Edge { to: from, kind });
    }

    /// Neighbors reachable in one hop from `id`, both directions, with the
    /// edge kind of each crossing.
    pub fn neighbors(&self, id: u64) -> Vec<(u64, &EdgeKind)> {
        let mut out: Vec<(u64, &EdgeKind)> = Vec::new();
        for e in self.forward.get(&id).map(|v| v.as_slice()).unwrap_or(&[]) {
            out.push((e.to, &e.kind));
        }
        for e in self.reverse.get(&id).map(|v| v.as_slice()).unwrap_or(&[]) {
            out.push((e.to, &e.kind));
        }
        out
    }
}
