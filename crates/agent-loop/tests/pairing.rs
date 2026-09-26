//! Tool-pairing invariant: a compaction cut never splits a
//! tool_use/tool_result pair (orbit invariant 1, dsh tool-pairing.ts).

use agent_loop::compaction::{Message, advance_to_balanced, is_balanced_at, select_compaction_cut};
use protocol::chat::{Block, Content, Role};

fn text(tag: usize) -> Message {
    Message::user_text(format!("{tag:04}{}", "x".repeat(996)))
}

fn assistant(ids: &[&str]) -> Message {
    Message {
        role: Role::Assistant,
        content: Content::Blocks(
            ids.iter()
                .map(|id| Block::ToolUse {
                    id: (*id).into(),
                    name: "echo_tool".into(),
                    input: serde_json::json!({}),
                })
                .collect(),
        ),
    }
}

fn results(ids: &[&str]) -> Message {
    Message {
        role: Role::User,
        content: Content::Blocks(
            ids.iter()
                .map(|id| Block::ToolResult {
                    tool_use_id: (*id).into(),
                    content: "done".into(),
                })
                .collect(),
        ),
    }
}

/// Independent orphan check (not the crate's own predicate): an id whose
/// tool_use sits on one side of the cut and whose tool_result on the other.
fn orphan_pair_ids(msgs: &[Message], cut: usize) -> Vec<String> {
    let mut orphans = vec![];
    for i in 0..msgs.len() {
        for (id, is_call) in block_ids(&msgs[i]) {
            let j = match is_call {
                true => find_pair(msgs, i, &id, false),
                false => find_pair(msgs, i, &id, true),
            };
            if let Some(j) = j
                && (i < cut) != (j < cut)
            {
                orphans.push(id);
            }
        }
    }
    orphans.sort();
    orphans.dedup();
    orphans
}

fn block_ids(m: &Message) -> Vec<(String, bool)> {
    let Content::Blocks(bs) = &m.content else {
        return vec![];
    };
    bs.iter()
        .map(|b| match b {
            Block::ToolUse { id, .. } => (id.clone(), true),
            Block::ToolResult { tool_use_id, .. } => (tool_use_id.clone(), false),
            Block::Text { .. } => (String::new(), false),
            Block::Image { .. } => (String::new(), false),
        })
        .filter(|(id, _)| !id.is_empty())
        .collect()
}

fn find_pair(msgs: &[Message], from: usize, id: &str, want_call: bool) -> Option<usize> {
    (0..msgs.len()).find(|&j| {
        j != from
            && block_ids(&msgs[j])
                .into_iter()
                .any(|(i, c)| i == id && c == want_call)
    })
}

/// Three tool pairs, every window size: whatever cut the policy produces,
/// no call/result pair is ever split — and no orphan survives.
#[test]
fn three_pairs_never_orphan_at_any_budget() {
    let msgs = vec![
        text(0),
        assistant(&["t1"]),
        results(&["t1"]),
        text(3),
        assistant(&["t2", "t3"]),
        results(&["t2"]),
        results(&["t3"]),
        text(7),
    ];
    assert_eq!(orphan_pair_ids(&msgs, msgs.len()), Vec::<String>::new());
    for budget in 0..=8_000 {
        let cut = select_compaction_cut(&msgs, budget);
        assert!(cut <= msgs.len());
        assert!(
            orphan_pair_ids(&msgs, cut).is_empty(),
            "budget {budget}: cut {cut} orphans a tool pair: {msgs:?}",
        );
        assert!(
            cut == msgs.len() || is_balanced_at(&msgs, cut),
            "budget {budget}: cut {cut} not balanced",
        );
    }
}

/// A cut landing mid-pair advances past the orphaned result(s).
#[test]
fn raw_unbalanced_cut_is_advanced() {
    let msgs = vec![text(0), assistant(&["t1"]), results(&["t1"]), text(3)];
    // index 2 starts on the kept result of a cut-away call → unbalanced.
    assert!(!is_balanced_at(&msgs, 2));
    assert!(is_balanced_at(&msgs, 3));
    assert_eq!(advance_to_balanced(&msgs, 2), 3);
    assert_eq!(advance_to_balanced(&msgs, msgs.len()), msgs.len());
}

/// Degenerate cuts stay degenerate instead of panicking.
#[test]
fn empty_context_has_no_pairings() {
    assert_eq!(advance_to_balanced(&[], 0), 0);
    assert!(is_balanced_at(&[], 0));
}
