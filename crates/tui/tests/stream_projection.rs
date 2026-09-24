//! stream_projection — 流式正文累进到 TurnEnd：delta 不拆行、不乱序、
//! 不丢尾（route §4 测试表第三条）。
//!
//! 对照 bug：把每片 delta 当独立段落渲染（每片一行）/ 转译层乱序 /
//! TurnEnd 收行时丢了最后一片。喂三片 delta + TurnEnd，逐条断言。

use omenic_tui::render_linear_line;
use web_state::ui_state::{AgentEvent, UiState};

/// 三片 delta 必须累成一句话、恰好一行，state 与回显一致，尾片不丢。
#[test]
fn streamed_text_accumulates_until_turn_end() {
    let mut state = UiState::default();
    let mut out: Vec<u8> = Vec::new();
    for delta in ["Hello, ", "wor", "ld!"] {
        let ev = AgentEvent::AssistantText {
            delta: delta.to_string(),
        };
        render_linear_line(&ev, &mut state, &mut out).expect("write to Vec cannot fail");
    }
    let end = AgentEvent::TurnEnd {
        stop_reason: "end_turn".to_string(),
    };
    render_linear_line(&end, &mut state, &mut out).expect("write to Vec cannot fail");
    let text = String::from_utf8(out).expect("linear output is utf-8");

    // 累进成整句：三片连续出现（junction 没被换行拆开 = 没乱序）。
    assert!(
        text.contains("Hello, world!"),
        "deltas split or reordered: {text:?}"
    );
    // 恰好一行：delta 之间无换行，TurnEnd 收恰好一行。
    assert_eq!(
        text.matches('\n').count(),
        1,
        "deltas must not start their own lines: {text:?}"
    );
    assert!(
        text.ends_with('\n'),
        "TurnEnd must close the line: {text:?}"
    );
    // 尾片（TurnEnd 前最后到达的 delta）没丢。
    assert!(text.contains("ld!"), "tail delta lost at TurnEnd: {text:?}");
    // 投影进 state 的正文与回显一致（last = assistant 占位已长出正文）。
    let content = &state.messages.last().expect("assistant message").content;
    assert_eq!(content, "Hello, world!");
}
