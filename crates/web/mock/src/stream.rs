//! `AgentEvent` 模拟流：把一次「发送」演成完整的 agent run。
//!
//! G4 时用 daemon `event.subscribe` 的接收端替换本模块，事件形状不变。

use std::time::Duration;

use omenic_web_state::ui_state::AgentEvent;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

/// 启动一个后台线程，按节奏推送一次模拟 run 的事件序列。
pub fn stream_reply(user_text: String) -> UnboundedReceiver<AgentEvent> {
    let (tx, rx) = unbounded_channel();
    std::thread::spawn(move || {
        let topic = user_text.replace('\n', " ");
        let topic = topic.trim();
        let topic = if topic.chars().count() > 32 {
            format!("{}…", topic.chars().take(32).collect::<String>())
        } else {
            topic.to_string()
        };
        let mut send = |ev: AgentEvent| {
            let _ = tx.send(ev);
        };

        send(AgentEvent::TurnStart);
        sleep_ms(350);

        stream_text(&send, "收到。我先定位与这个需求相关的代码，再决定改法。");
        sleep_ms(250);

        // 第一件工具：bash 探查
        let tc_id = "mock-tc-1".to_string();
        send(AgentEvent::ToolCall {
            id: tc_id.clone(),
            name: "run_bash".into(),
            args: serde_json::json!({ "command": format!("rg -n '{topic}' --type rust -l") }),
        });
        sleep_ms(350);
        send(AgentEvent::ToolStart { id: tc_id.clone() });
        sleep_ms(600);
        send(AgentEvent::ToolResult {
            id: tc_id,
            name: "run_bash".into(),
            result: "crates/web/page-workspace/src/lib.rs\ncrates/web/components/src/chat.rs\n2 个文件命中".into(),
        });

        stream_text(&send, "相关逻辑集中在 `page-workspace` 与 `chat` 组件里。我先对输入入口做一次小改动，把事件处理收敛到转译层。");
        sleep_ms(250);

        // 第二件工具：edit（带 diff 输出）
        let tc_id = "mock-tc-2".to_string();
        send(AgentEvent::ToolCall {
            id: tc_id.clone(),
            name: "edit".into(),
            args: serde_json::json!({ "path": "crates/web/page-workspace/src/lib.rs" }),
        });
        sleep_ms(350);
        send(AgentEvent::ToolStart { id: tc_id.clone() });
        sleep_ms(700);
        send(AgentEvent::ToolResult {
            id: tc_id,
            name: "edit".into(),
            result: "@@ -12,6 +12,10 @@\n+    let mut ui = UiState::default();\n+    ui.apply(&ev);\n-    // TODO: 接线后删除\n patch applied".into(),
        });

        stream_text(&send, "改完了。事件现在统一走 `UiState::apply`，后面接 daemon 事件流时只需要替换 transport。");
        sleep_ms(200);
        stream_text(&send, "总结：\n\n1. 输入 → `AgentEvent` 模拟流\n2. 转译层逐事件更新 UI 状态\n3. 渲染按发生顺序展开\n\n如需继续，可以直接说下一步要调整的地方。");

        send(AgentEvent::TurnEnd {
            stop_reason: "end_turn".into(),
        });
    });
    rx
}

fn stream_text(send: &impl Fn(AgentEvent), text: &str) {
    // 以 6 字符粒度推 delta，近似真实流式节奏
    let chars: Vec<char> = text.chars().collect();
    for chunk in chars.chunks(6) {
        let delta: String = chunk.iter().collect();
        send(AgentEvent::AssistantText { delta });
        sleep_ms(45);
    }
}

fn sleep_ms(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}
