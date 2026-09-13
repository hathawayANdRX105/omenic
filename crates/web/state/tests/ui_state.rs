//! C5.1 转译层测试：serde 往返 + 事件序列 → UI 状态。

use omenic_web_state::types::MessagePart;
use omenic_web_state::ui_state::{AgentEvent, UiState};
use serde_json::json;

#[test]
fn agent_event_serde_round_trip() {
    let events = vec![
        AgentEvent::TurnStart,
        AgentEvent::AssistantText {
            delta: "你好".into(),
        },
        AgentEvent::ToolCall {
            id: "tc-1".into(),
            name: "run_bash".into(),
            args: json!({ "command": "cargo test" }),
        },
        AgentEvent::ToolStart { id: "tc-1".into() },
        AgentEvent::ToolResult {
            id: "tc-1".into(),
            name: "run_bash".into(),
            result: "test result: ok".into(),
        },
        AgentEvent::TurnEnd {
            stop_reason: "end_turn".into(),
        },
    ];
    for ev in events {
        let text = serde_json::to_string(&ev).unwrap();
        let back: AgentEvent = serde_json::from_str(&text).unwrap();
        assert_eq!(back, ev, "round-trip failed for {text}");
    }
}

#[test]
fn streamed_sequence_builds_chronological_parts() {
    let mut ui = UiState::default();
    ui.push_message(omenic_web_state::types::ChatMessage {
        id: "m1".into(),
        role: "user".into(),
        content: "跑一下测试".into(),
        tool_calls: vec![],
        parts: vec![],
        timestamp: "刚刚".into(),
        ts_epoch_ms: 0,
    });

    for ev in [
        AgentEvent::AssistantText {
            delta: "好的，".into(),
        },
        AgentEvent::AssistantText {
            delta: "先看目录。".into(),
        },
        AgentEvent::ToolCall {
            id: "tc-1".into(),
            name: "run_bash".into(),
            args: json!({ "command": "ls" }),
        },
        AgentEvent::ToolResult {
            id: "tc-1".into(),
            name: "run_bash".into(),
            result: "src".into(),
        },
        AgentEvent::AssistantText {
            delta: "完成。".into(),
        },
        AgentEvent::TurnEnd {
            stop_reason: "end_turn".into(),
        },
    ] {
        ui.apply(&ev);
    }

    // assistant 占位自动补出（用户消息之后）
    assert_eq!(ui.messages.len(), 2);
    let asst = &ui.messages[1];
    assert_eq!(asst.content, "好的，先看目录。完成。");
    // 发生顺序：文本段 → 工具 → 文本段
    assert_eq!(asst.parts.len(), 3);
    assert!(matches!(&asst.parts[0], MessagePart::Text(t) if t == "好的，先看目录。"));
    assert!(matches!(&asst.parts[1], MessagePart::Tool(tc) if tc.status == "success"));
    assert!(matches!(&asst.parts[2], MessagePart::Text(t) if t == "完成。"));
}

#[test]
fn tool_call_extracts_title_and_kind() {
    let mut ui = UiState::default();
    ui.apply(&AgentEvent::ToolCall {
        id: "tc-2".into(),
        name: "edit".into(),
        args: json!({ "path": "crates/orbit/src/lib.rs" }),
    });
    let asst = &ui.messages[0];
    let MessagePart::Tool(tc) = &asst.parts[0] else {
        panic!("expected tool part");
    };
    assert_eq!(tc.kind, "edit");
    assert_eq!(tc.title, "crates/orbit/src/lib.rs");
    assert_eq!(tc.status, "running");
}

#[test]
fn empty_turn_end_writes_placeholder_text() {
    let mut ui = UiState::default();
    ui.apply(&AgentEvent::TurnEnd {
        stop_reason: "max_turns".into(),
    });
    assert_eq!(ui.messages.len(), 1);
    assert!(ui.messages[0].content.contains("max_turns"));
}
