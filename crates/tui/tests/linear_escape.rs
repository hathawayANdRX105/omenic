//! linear_escape — route §8 禁止项钉子：linear 输出零转义字节。
//!
//! 对照 route §8：T1/T4 禁止 CSI/OSC/ESC 系列；光标寻址、颜色 SGR、OSC
//! （超链接）都藏在这些前缀里。真实事件全家（reasoning / 正文 / tool
//! start/call/result + 两轮 TurnEnd，含空 turn 占位）过一遍
//! [`render_linear_line`]，产出字节里连 ESC（0x1b）都不许有——模型内容里
//! 注入的转义序列必须被 sanitize 滤掉，渲染器自己也不写任何转义序列。
//! enhanced 恒走另一条渲染路径，改不掉这条（route §8 的 TUI/C-Sem 恒保留）。

use omenic_tui::render_linear_line;
use web_state::ui_state::{AgentEvent, UiState};

/// ESC——所有 C0 转义的引导字节；出现即违反 route §8。
const ESC: u8 = 27;

/// 一串事件过 linear 投影，返回累积输出字节（state 跨事件保持，模拟真实流）。
fn project(events: &[AgentEvent]) -> Vec<u8> {
    let mut state = UiState::default();
    let mut out: Vec<u8> = Vec::new();
    for ev in events {
        render_linear_line(ev, &mut state, &mut out).expect("write to Vec never fails");
    }
    out
}

/// 各类事件（含 reasoning/tool 全家 + 空 turn 占位）投影后零 ESC 字节。
#[test]
fn linear_output_emits_zero_escape_bytes() {
    // 一轮有正文 + 工具的完整 turn：各事件内容都塞转义序列（CSI 色码、
    // OSC 超链接、清屏序列），投影必须全部滤掉。
    let mut bytes = project(&[
        AgentEvent::TurnStart,
        AgentEvent::Reasoning {
            delta: "thinking \u{1b}[31mred-ish".to_string(),
        },
        AgentEvent::AssistantText {
            delta: "hello \u{1b}]8;;http://x\u{1b}\\link\u{1b}[0m".to_string(),
        },
        AgentEvent::ToolStart { id: "c1".into() },
        AgentEvent::ToolCall {
            id: "c1".into(),
            name: "run_bash".into(),
            args: serde_json::json!({ "command": "ls \u{1b}[2J" }),
        },
        AgentEvent::ToolResult {
            id: "c1".into(),
            name: "run_bash".into(),
            result: "file.txt\n\u{1b}[0m".into(),
        },
        AgentEvent::TurnEnd {
            stop_reason: "end_turn".into(),
        },
    ]);
    // 空 turn（无正文无工具）：占位正文嵌 stop_reason，同样必须零 ESC。
    bytes.extend(project(&[
        AgentEvent::TurnStart,
        AgentEvent::TurnEnd {
            stop_reason: "error\u{1b}[31m".into(),
        },
    ]));

    assert!(
        !bytes.contains(&ESC),
        "linear output must emit zero escape bytes (route §8), got {:?}",
        String::from_utf8_lossy(&bytes)
    );
    // 顺带钉：无 ESC 的前提下投影仍有可见产出（正文、工具行、占位都在）。
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("hello"), "assistant text missing: {text:?}");
    assert!(text.contains("[tool]"), "tool line missing: {text:?}");
    assert!(
        text.contains("Agent"),
        "empty-turn placeholder missing: {text:?}"
    );
}
