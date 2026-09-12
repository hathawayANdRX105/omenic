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
#[derive(Debug, Default, Clone)]
pub struct MemoryGraph {
    pub memories: HashMap<u64, MemoryEntry>,
    /// Forward adjacency: entry → outgoing edges.
    pub forward: HashMap<u64, Vec<Edge>>,
    /// Reverse adjacency (entry → incoming edge sources) for backlink walks.
    pub reverse: HashMap<u64, Vec<u64>>,
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
        for e in entries {
            // Tag edges are undirected in spirit; one mirrored edge per tag
            // match, sorted by id so the built graph is deterministic. Tags
            // are cloned out first: `link` needs `&mut graph` and the scan
            // borrows it.
            let mut by_tag: HashMap<String, Vec<u64>> = HashMap::new();
            for (other_id, other) in graph.memories.iter() {
                if *other_id == e.id {
                    continue;
                }
                for tag in &other.tags {
                    if e.tags.contains(tag) {
                        by_tag.entry(tag.clone()).or_default().push(*other_id);
                    }
                }
            }
            let mut matches: Vec<(String, u64)> = by_tag
                .into_iter()
                .flat_map(|(tag, targets)| targets.into_iter().map(move |t| (tag.clone(), t)))
                .collect();
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
        self.forward
            .entry(from)
            .or_default()
            .push(Edge { to, kind });
        self.reverse.entry(to).or_default().push(from);
    }

    /// Neighbors reachable in one hop from `id`, both directions.
    pub fn neighbors(&self, id: u64) -> Vec<(u64, &EdgeKind)> {
        let mut out: Vec<(u64, &EdgeKind)> = Vec::new();
        if let Some(edges) = self.forward.get(&id) {
            for e in edges {
                out.push((e.to, &e.kind));
            }
        }
        // Reverse-walk kind: rebuilt from the source's forward edge.
        for src in self
            .reverse
            .get(&id)
            .map(|v| v.as_slice())
            .unwrap_or_default()
        {
            if let Some(edges) = self.forward.get(src) {
                for e in edges {
                    if e.to == id {
                        out.push((*src, &e.kind));
                    }
                }
            }
        }
        out
    }
}
