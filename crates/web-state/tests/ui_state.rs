//! C5.1 转译层测试：serde 往返 + 事件序列 → UI 状态。

use serde_json::json;
use web_state::types::MessagePart;
use web_state::ui_state::{AgentEvent, UiState};

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
    ui.push_message(web_state::types::ChatMessage {
        id: "m1".into(),
        role: "user".into(),
        content: "跑一下测试".into(),
        reasoning: String::new(),
        reasoning_started_ms: None,
        reasoning_ms: None,
        tool_calls: vec![],
        parts: vec![],
        timestamp: "刚刚".into(),
        ts_epoch_ms: 0,
        attachments: vec![],
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
fn reasoning_segments_interleave_in_stream_order() {
    let mut ui = UiState::default();
    ui.push_message(web_state::types::ChatMessage {
        id: "m1".into(),
        role: "user".into(),
        content: "证明 1+1=2".into(),
        reasoning: String::new(),
        reasoning_started_ms: None,
        reasoning_ms: None,
        tool_calls: vec![],
        parts: vec![],
        timestamp: "刚刚".into(),
        ts_epoch_ms: 0,
        attachments: vec![],
    });

    for ev in [
        // 第一段思考：相邻 delta 归同一段
        AgentEvent::Reasoning {
            delta: "先想公理。".into(),
        },
        AgentEvent::Reasoning {
            delta: "再加结合律。".into(),
        },
        // 工具边界切段
        AgentEvent::ToolCall {
            id: "tc-1".into(),
            name: "run_bash".into(),
            args: json!({ "command": "echo qed" }),
        },
        AgentEvent::ToolResult {
            id: "tc-1".into(),
            name: "run_bash".into(),
            result: "qed".into(),
        },
        // 第二段思考（同一轮内再次思考）
        AgentEvent::Reasoning {
            delta: "收尾检查。".into(),
        },
        // 文本边界再切段
        AgentEvent::AssistantText {
            delta: "得证。".into(),
        },
        AgentEvent::TurnEnd {
            stop_reason: "end_turn".into(),
        },
    ] {
        ui.apply(&ev);
    }

    let asst = &ui.messages[1];
    // 消息级思考累积（TUI 兼容口径）不被 parts 化影响
    assert_eq!(asst.reasoning, "先想公理。再加结合律。收尾检查。");
    // 发生顺序：思考段 → 工具 → 思考段 → 文本段
    assert_eq!(asst.parts.len(), 4);
    let MessagePart::Reasoning {
        text,
        duration_ms: d1,
        ..
    } = &asst.parts[0]
    else {
        panic!("parts[0]: expected reasoning segment");
    };
    assert_eq!(text, "先想公理。再加结合律。");
    assert!(d1.is_some(), "closed segment must carry a duration");
    assert!(matches!(asst.parts[1], MessagePart::Tool(_)));
    let MessagePart::Reasoning { text, .. } = &asst.parts[2] else {
        panic!("parts[2]: expected second reasoning segment");
    };
    assert_eq!(text, "收尾检查。");
    assert!(matches!(&asst.parts[3], MessagePart::Text(t) if t == "得证。"));
}

#[test]
fn active_reasoning_segment_stays_open_until_boundary() {
    let mut ui = UiState::default();
    ui.apply(&AgentEvent::Reasoning {
        delta: "思考中…".into(),
    });
    let asst = &ui.messages[0];
    let MessagePart::Reasoning {
        started_ms,
        duration_ms,
        ..
    } = &asst.parts[0]
    else {
        panic!("expected reasoning part");
    };
    assert!(started_ms.is_some(), "first delta stamps segment start");
    assert!(duration_ms.is_none(), "active segment unsettled");

    // 文本边界结算
    ui.apply(&AgentEvent::AssistantText {
        delta: "作答".into(),
    });
    let asst = &ui.messages[0];
    let MessagePart::Reasoning { duration_ms, .. } = &asst.parts[0] else {
        panic!("expected reasoning part");
    };
    assert!(duration_ms.is_some(), "boundary event settles the segment");
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

/// 后台作业与持久终端的工具名归一化。
///
/// 这条测试守的是"未知工具走兜底"这个默认行为的边界：jobs_* / terminal_*
/// 的参数里没有 `path`，一旦落进兜底分支，气泡标题就会退化成工具名本身，
/// 聊天气泡上看不出这一步在做什么。所以每个名字都必须映射到
/// bash / job / terminal 三个 kind 之一，并且标题取自 id / data / command。
#[test]
fn job_and_terminal_tools_get_a_specific_kind_and_a_useful_title() {
    let cases = [
        (
            "jobs_start",
            json!({ "command": "cargo test" }),
            "bash",
            "cargo test",
        ),
        ("jobs_wait", json!({ "id": "job-3" }), "job", "job-3"),
        ("jobs_list", json!({}), "job", "jobs_list"),
        ("jobs_kill", json!({ "id": "job-3" }), "job", "job-3"),
        (
            "terminal_create",
            json!({ "shell": "bash" }),
            "terminal",
            "terminal_create",
        ),
        (
            "terminal_write",
            json!({ "id": "term-1", "data": "pwd\n" }),
            "terminal",
            "term-1",
        ),
        (
            "terminal_read",
            json!({ "id": "term-1" }),
            "terminal",
            "term-1",
        ),
        (
            "terminal_resize",
            json!({ "id": "term-1", "cols": 100, "rows": 30 }),
            "terminal",
            "term-1",
        ),
        (
            "terminal_kill",
            json!({ "id": "term-1" }),
            "terminal",
            "term-1",
        ),
        ("terminal_list", json!({}), "terminal", "terminal_list"),
    ];

    for (name, args, want_kind, want_title) in cases {
        let mut ui = UiState::default();
        ui.apply(&AgentEvent::ToolCall {
            id: "tc-1".into(),
            name: name.into(),
            args,
        });
        let MessagePart::Tool(tc) = &ui.messages[0].parts[0] else {
            panic!("{name}: expected a tool part");
        };
        assert_eq!(tc.kind, want_kind, "{name}: wrong kind");
        assert_eq!(tc.title, want_title, "{name}: wrong title");
        assert_ne!(
            tc.kind, "tool",
            "{name}: fell through to the unknown-tool fallback"
        );
    }
}
