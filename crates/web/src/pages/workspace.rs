use crate::components::chat::Chat;
use crate::components::sidebar::Sidebar;
use crate::components::taskpanel::TaskPanel;
use crate::mock::{self, ChatMessage, Session, SessionStatus, ToolCall};
use dioxus::prelude::*;
use std::collections::HashMap;

#[component]
pub fn Workspace() -> Element {
    let mut sessions = use_signal(mock::mock_sessions);
    let mut active_session_id = use_signal(|| "s1".to_string());
    let mut session_messages = use_signal(|| {
        let mut map: HashMap<String, Vec<ChatMessage>> = HashMap::new();
        map.insert("s1".into(), mock::mock_messages_for_session("s1"));
        map.insert("s2".into(), mock::mock_messages_for_session("s2"));
        map.insert("s3".into(), mock::mock_messages_for_session("s3"));
        map
    });

    let mut statusline = use_signal(mock::mock_statusline);
    let tasks = mock::mock_tasks();

    let current_messages = session_messages()
        .get(&active_session_id())
        .cloned()
        .unwrap_or_else(|| mock::mock_messages_for_session(&active_session_id()));

    // Handler: Send Message
    let on_send = move |text: String| {
        let sid = active_session_id();
        let existing_len = session_messages().get(&sid).map(|m| m.len()).unwrap_or(0);
        let user_id = format!("{}-user-{}", sid, existing_len + 1);
        let assistant_id = format!("{}-asst-{}", sid, existing_len + 2);

        let user_msg = ChatMessage {
            id: user_id,
            role: "user".into(),
            content: text.clone(),
            tool_calls: vec![],
            timestamp: "刚刚".into(),
        };

        // Synthesize dynamic tool calls to showcase collapsible tool inspection
        let assistant_msg = ChatMessage {
            id: assistant_id,
            role: "assistant".into(),
            content: format!("已收到指令：`{text}`\n正在解析并调用系统工具执行任务..."),
            tool_calls: vec![
                ToolCall {
                    id: format!("tc-run-{}", sid),
                    title: format!("已执行 bash 探索：find_relevant_code(\"{text}\")"),
                    kind: "bash".into(),
                    summary: "检索到 2 个目标符号定义，退出码 0".into(),
                    detail: format!("$ rg --json \"{text}\"\n{{\"type\":\"match\",\"data\":{{\"path\":\"crates/web/src/workspace.rs\",\"line_number\":42}}}}\nexit status: 0"),
                    status: "success".into(),
                },
                ToolCall {
                    id: format!("tc-edit-{}", sid),
                    title: "已拟定 patch 并写入代码缓冲区".into(),
                    kind: "edit".into(),
                    summary: "就绪待确认，影响 1 个文件".into(),
                    detail: format!("--- a/src/target.rs\n+++ b/src/target.rs\n@@ -10,3 +10,4 @@\n+    // Auto-generated response for {text}\n"),
                    status: "success".into(),
                },
            ],
            timestamp: "刚刚".into(),
        };

        let mut map = session_messages();
        let list = map.entry(sid.clone()).or_insert_with(Vec::new);
        list.push(user_msg);
        list.push(assistant_msg);
        session_messages.set(map);

        // Update status line tokens and cost
        let mut st = statusline();
        st.tokens_in += 42;
        st.tokens_out += 180;
        st.cost_usd += 0.001;
        statusline.set(st);
    };

    // Handler: Switch Session
    let on_select_session = move |id: String| {
        active_session_id.set(id);
    };

    // Handler: Create New Session
    let on_create_session = move |()| {
        let new_num = sessions().len() + 1;
        let new_id = format!("s{}", new_num);
        let new_session = Session {
            id: new_id.clone(),
            title: format!("新会话 #{}", new_num),
            last_active: "刚刚".into(),
            model: statusline().model.clone(),
            status: SessionStatus::Active,
        };
        let mut current_sessions = sessions();
        current_sessions.insert(0, new_session);
        sessions.set(current_sessions);

        let mut map = session_messages();
        map.insert(
            new_id.clone(),
            vec![ChatMessage {
                id: format!("{}-welcome", new_id),
                role: "assistant".into(),
                content: format!(
                    "新会话已创建。当前配置模型为 `{}`，输入指令开始对话。",
                    statusline().model
                ),
                tool_calls: vec![],
                timestamp: "刚刚".into(),
            }],
        );
        session_messages.set(map);
        active_session_id.set(new_id);
    };

    // Handler: Switch Model
    let on_model_change = move |m: String| {
        let mut st = statusline();
        st.model = m;
        statusline.set(st);
    };

    // Handler: Toggle Thinking
    let on_toggle_thinking = move |()| {
        let mut st = statusline();
        st.thinking = if st.thinking == "off" {
            "medium (2048)".into()
        } else if st.thinking.starts_with("medium") {
            "high (8192)".into()
        } else {
            "off".into()
        };
        statusline.set(st);
    };

    rsx! {
        div { class: "workspace-layout",
            Sidebar {
                sessions: sessions(),
                active_id: active_session_id(),
                on_select: on_select_session,
                on_create: on_create_session,
            }
            div { class: "workspace-main",
                div { class: "workspace-chat-area",
                    Chat {
                        messages: current_messages,
                        statusline: statusline(),
                        on_send: on_send,
                        on_model_change: on_model_change,
                        on_toggle_thinking: on_toggle_thinking,
                    }
                }
                div { class: "workspace-side",
                    TaskPanel { tasks: tasks }
                }
            }
        }
    }
}
