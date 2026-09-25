//! Recall pipeline: score entries against a query, then expand through the
//! derived graph.
//!
//! jcode lesson baked in: the candidate pool stays wide (hard cosine-style
//! thresholds destroyed its recall@5 — 0.0 at 0.5), precision is the
//! reranker's job later. Scoring here is tokenizer + idf-weighted overlap,
//! no embedding yet; the graph cascade (`score × edge_weight × 0.7^depth`)
//! only ever adds candidates, never filters them.

use std::collections::HashMap;

use crate::graph::MemoryGraph;

/// Query and entry texts share one tokenizer: ASCII words, CJK chars and
/// character bigrams (no jieba — bigrams carry enough discrimination for a
/// local store and keep the crate dependency-free).
pub fn tokenize(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut tokens = Vec::new();
    let chars = lower.chars();
    let mut ascii_word = String::new();
    let mut prev_cjk: Option<char> = None;
    for c in chars {
        if c.is_ascii_alphanumeric() {
            ascii_word.push(c);
            prev_cjk = None;
            continue;
        }
        if !ascii_word.is_empty() {
            tokens.push(std::mem::take(&mut ascii_word));
        }
        if is_cjk(c) {
            tokens.push(c.to_string());
            if let Some(p) = prev_cjk {
                let mut bigram = String::new();
                bigram.push(p);
                bigram.push(c);
                tokens.push(bigram);
            }
            prev_cjk = Some(c);
        } else {
            prev_cjk = None;
        }
    }
    if !ascii_word.is_empty() {
        tokens.push(ascii_word);
    }
    tokens
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF)
}

/// One retrieval result.
#[derive(Debug, Clone, PartialEq)]
pub struct RecallHit {
    pub id: u64,
    /// Direct score + graph cascade bonus, in [0, ∞).
    pub score: f32,
    /// How the entry was reached: direct match, or via which entry/kind.
    pub via: RecallPath,
    /// The entry's text, cloned for the returned top-k only — consumers
    /// (tools, injection formatting) need it and must not re-read the store.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecallPath {
    Direct,
    /// Reached through `from` over an edge (cascade depth ≥ 1).
    Graph {
        from: u64,
        depth: u8,
    },
}

/// Per-entry token frequency, computed once per recall pass: both the IDF
/// table and the direct scoring read from this, instead of re-tokenizing
/// per query term.
struct Tokenized {
    id: u64,
    active: bool,
    tf: HashMap<String, u32>,
}

fn tokenize_entries(graph: &MemoryGraph) -> Vec<Tokenized> {
    graph
        .memories
        .values()
        .map(|entry| {
            let mut tf: HashMap<String, u32> = HashMap::new();
            for tok in tokenize(&entry.text) {
                *tf.entry(tok).or_default() += 1;
            }
            Tokenized {
                id: entry.id,
                active: entry.active,
                tf,
            }
        })
        .collect()
}

/// Top-`k` entries for `query`. Direct matches ranked by idf-weighted token
/// overlap, then one graph cascade: neighbors inherit
/// `score × edge_weight × 0.7^depth` and compete for the same slots.
///
/// An empty query yields no direct seeds, and the cascade only runs from
/// seeds, so the result is empty — callers that want "recent" should use
/// [`crate::Memory::list`].
pub fn recall(graph: &MemoryGraph, query: &str, k: usize) -> Vec<RecallHit> {
    let docs = tokenize_entries(graph);
    let query_tokens = tokenize(query);
    let mut scores: HashMap<u64, (f32, RecallPath)> = HashMap::new();

    if query_tokens.is_empty() {
        return Vec::new();
    }

    // IDF over the deduplicated document frequency of every token.
    let n = docs.len().max(1) as f32;
    let mut df: HashMap<&str, u32> = HashMap::new();
    for doc in &docs {
        for tok in doc.tf.keys() {
            *df.entry(tok).or_default() += 1;
        }
    }
    let idf = |tok: &str| {
        df.get(tok)
            .map(|d| ((n + 1.0) / (*d as f32 + 0.5)).ln().max(0.05))
            .unwrap_or(0.05)
    };

    for doc in &docs {
        // Inactive entries are dead content (superseded/contradicted):
        // jcode keeps them out of the direct path and reachable only
        // through the graph walk, so a dead claim never outranks its
        // live replacement on its old wording.
        if !doc.active {
            continue;
        }
        let mut overlap = 0f32;
        for q in &query_tokens {
            let tf = doc.tf.get(q).copied().unwrap_or(0) as f32;
            if tf > 0.0 {
                overlap += idf(q) * (1.0 + tf.ln());
            }
        }
        if overlap > 0.0 {
            scores.insert(doc.id, (overlap, RecallPath::Direct));
        }
    }

    // Cascade: breadth-first over edges, two hops max. An inherited score
    // only replaces an existing one when strictly better; walking into an
    // inactive (superseded) entry is discounted so dead history can't
    // outrank its live replacement.
    const DECAY: f32 = 0.7;
    let mut frontier: Vec<(u64, f32)> = scores.iter().map(|(id, (s, _))| (*id, *s)).collect();
    let mut depth = 0u8;
    while depth < 2 {
        depth += 1;
        let mut next: Vec<(u64, f32)> = Vec::new();
        for (id, score) in &frontier {
            for (to, kind) in graph.neighbors(*id) {
                let mut inherit = score * kind.traversal_weight() * DECAY.powi(depth as i32);
                if graph.memories.get(&to).is_some_and(|t| !t.active) {
                    inherit *= 0.5;
                }
                let path = RecallPath::Graph { from: *id, depth };
                match scores.get(&to) {
                    Some((existing, _)) if *existing >= inherit => {}
                    _ => {
                        scores.insert(to, (inherit, path));
                        next.push((to, inherit));
                    }
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }

    let mut scored: Vec<(u64, f32, RecallPath)> = scores
        .into_iter()
        .filter(|(id, _)| graph.memories.contains_key(id))
        .map(|(id, (score, via))| (id, score, via))
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    scored.truncate(k);
    // Clone texts only for the returned slice.
    scored
        .into_iter()
        .map(|(id, score, via)| RecallHit {
            id,
            score,
            via,
            text: graph.memories[&id].text.clone(),
        })
        .collect()
}
