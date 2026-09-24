//! Tool-pairing balance across a compaction cut.
//!
//! Reference: dsh `compaction/compaction/src/tool-pairing.ts` — a per-event
//! `+1` (tool-call) / `-1` (tool-result) balance fold answering
//! `toolPairingBalancedBefore/After`. Ported to the message-window model:
//! a cut is balanced when the removed prefix contains whole
//! tool_use/tool_result pairs, i.e. deleting a call always deletes its
//! result and vice versa (orbit invariant 1: every tool_call in the shipped
//! context must carry its matching tool_result).

use std::collections::HashSet;

use protocol::chat::{Block, Content};

use crate::Message;

fn ids(m: &Message, tool_use: bool) -> Vec<&str> {
    let Content::Blocks(bs) = &m.content else {
        return Vec::new();
    };
    bs.iter()
        .filter_map(|b| match (b, tool_use) {
            (Block::ToolUse { id, .. }, true) => Some(id.as_str()),
            (Block::ToolResult { tool_use_id, .. }, false) => Some(tool_use_id.as_str()),
            _ => None,
        })
        .collect()
}

/// Whether the cut at `index` keeps every tool_use/tool_result pair intact:
/// no result in the kept window answers a call being compacted away. A kept
/// call can never lose its result to the prefix (results only ever follow
/// their call), so this single check covers both directions.
pub fn is_balanced_at(messages: &[Message], index: usize) -> bool {
    debug_assert!(index <= messages.len());
    let prefix_calls: HashSet<&str> = messages[..index]
        .iter()
        .flat_map(|m| ids(m, true))
        .collect();
    !messages[index..].iter().any(|m| {
        ids(m, false)
            .into_iter()
            .any(|id| prefix_calls.contains(id))
    })
}

/// Advance a raw window cut forward until the kept window opens
/// pairing-balanced (or the end of the context is reached).
///
/// ponytail: recomputes the prefix set per step; the message count after a
/// char-budget cut makes this a rounding error next to the char scan.
pub fn advance_to_balanced(messages: &[Message], mut cut: usize) -> usize {
    while cut < messages.len() && !is_balanced_at(messages, cut) {
        cut += 1;
    }
    cut
}
