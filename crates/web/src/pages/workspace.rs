use crate::components::chat::Chat;
use crate::components::sidebar::Sidebar;
use crate::components::taskpanel::TaskPanel;
use crate::llm::LlmRuntimeConfig;
use crate::mock::{self, ChatMessage, Session, SessionStatus, ToolCall};
use dioxus::prelude::*;
use std::collections::HashMap;

#[component]
pub fn Workspace(
    config: LlmRuntimeConfig,
    on_update_config: EventHandler<LlmRuntimeConfig>,
) -> Element {
    let mut sessions = use_signal(mock::mock_sessions);
    let mut active_session_id = use_signal(|| "s1".to_string());
    let mut session_messages = use_signal(|| {
        let mut map: HashMap<String, Vec<ChatMessage>> = HashMap::new();
        map.insert("s1".into(), mock::mock_messages_for_session("s1"));
        map.insert("s2".into(), mock::mock_messages_for_session("s2"));
        map.insert("s3".into(), mock::mock_messages_for_session("s3"));
        map
    });

    let mut statusline = use_signal(|| {
        let mut st = mock::mock_statusline();
        st.model = config.model.clone();
        st
    });

    let tasks = mock::mock_tasks();
    let mut show_tasks = use_signal(|| true);
    let mut is_streaming = use_signal(|| false);

    let active_title = sessions()
        .iter()
        .find(|s| s.id == active_session_id())
        .map(|s| s.title.clone())
        .unwrap_or_else(|| "当前对话".into());

    let current_messages = session_messages()
        .get(&active_session_id())
        .cloned()
        .unwrap_or_else(|| mock::mock_messages_for_session(&active_session_id()));

    let config_send = config.clone();
    let config_create = config.clone();
    let config_model = config.clone();

    // Handler: Send Message with Real LLM Fallback
    let on_send = move |text: String| {
        let sid = active_session_id();
        let existing_len = session_messages().get(&sid).map(|m| m.len()).unwrap_or(0);
        let user_id = format!("{}-user-{}", sid, existing_len + 1);
        let asst_id = format!("{}-asst-{}", sid, existing_len + 2);

        let user_msg = ChatMessage {
            id: user_id,
            role: "user".into(),
            content: text.clone(),
            tool_calls: vec![],
            timestamp: "刚刚".into(),
        };

        let history = {
            let mut map = session_messages();
            let list = map.entry(sid.clone()).or_insert_with(Vec::new);
            list.push(user_msg.clone());
            let h = list.clone();
            session_messages.set(map);
            h
        };

        is_streaming.set(true);

        // Prepare context for real LLM request
        let cfg_clone = config_send.clone();
        // Perform real LLM call
        spawn(async move {
            let start = std::time::Instant::now();
            let result = cfg_clone.chat(&history);
            let elapsed = start.elapsed();

            let (asst_content, tool_calls) = match result {
                Ok(reply) => {
                    let tc = vec![ToolCall {
                        id: format!("tc-live-{}", sid),
                        title: format!("▶ 已完成模型推理: {} ({:.1}s)", cfg_clone.model, elapsed.as_secs_f64()),
                        kind: "bash".into(),
                        summary: format!("耗时 {:.2}s，模型 {} 正常输出", elapsed.as_secs_f64(), cfg_clone.model),
                        detail: format!(
                            "端点: {}/v1/chat/completions\n模型: {}\n耗时: {:.2}s\n返回长度: {} 字符\n状态: HTTP 200 OK",
                            cfg_clone.base_url.trim_end_matches('/'),
                            cfg_clone.model,
                            elapsed.as_secs_f64(),
                            reply.len()
                        ),
                        status: "success".into(),
                    }];
                    (reply, tc)
                }
                Err(err) => {
                    let tc = vec![ToolCall {
                        id: format!("tc-err-{}", sid),
                        title: format!("⚠️ 无法直连真实端点: {}", cfg_clone.base_url),
                        kind: "edit".into(),
                        summary: format!("错误: {} (请在「配置」页检查 API Key / URL)", err),
                        detail: format!("请求错误信息: {}\n当前端点: {}\n已自动降级为本地 mock 回复，确保界面可用。", err, cfg_clone.base_url),
                        status: "error".into(),
                    }];
                    (format!("收到指令：`{text}`\n\n（提示：当前连接到 `{}` 发生异常：{}。已自动降级展示）", cfg_clone.base_url, err), tc)
                }
            };

            let asst_msg = ChatMessage {
                id: asst_id,
                role: "assistant".into(),
                content: asst_content,
                tool_calls,
                timestamp: "刚刚".into(),
            };

            let mut map = session_messages();
            if let Some(list) = map.get_mut(&sid) {
                list.push(asst_msg);
            }
            session_messages.set(map);

            let mut st = statusline();
            st.tokens_in += 64;
            st.tokens_out += 220;
            st.cost_usd += 0.002;
            statusline.set(st);

            is_streaming.set(false);
        });
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
            model: config_create.model.clone(),
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
                    "新会话已开启。当前连接模型为 `{}` (端点 `{}`)，请输入任务开始工作。",
                    config_create.model, config_create.base_url
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
        st.model = m.clone();
        statusline.set(st);

        let mut cfg = config_model.clone();
        cfg.model = m;
        on_update_config.call(cfg);
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
            // Left Sidebar
            Sidebar {
                sessions: sessions(),
                active_id: active_session_id(),
                on_select: on_select_session,
                on_create: on_create_session,
            }

            // Center Chat Canvas
            div { class: "workspace-main",
                div { class: "workspace-chat-area",
                    // Header Bar (zcode breadcrumbs & taskboard toggle)
                    div { class: "chat-header-bar",
                        div { class: "chat-header-title",
                            span { "💬" }
                            span { "{active_title}" }
                            span { class: "chat-header-badge", "{config.model}" }
                        }
                        button {
                            class: if show_tasks() { "btn-toggle-taskpanel active" } else { "btn-toggle-taskpanel" },
                            onclick: move |_| show_tasks.set(!show_tasks()),
                            if show_tasks() { "📋 隐藏看板" } else { "📋 任务看板 (7)" }
                        }
                    }

                    // Chat Stream
                    Chat {
                        messages: current_messages,
                        statusline: statusline(),
                        is_streaming: is_streaming(),
                        on_send: on_send,
                        on_model_change: on_model_change,
                        on_toggle_thinking: on_toggle_thinking,
                    }
                }

                // Right Panel (zcode Goal & Progress)
                if show_tasks() {
                    TaskPanel {
                        tasks: tasks,
                        on_close: move |_| show_tasks.set(false),
                    }
                }
            }
        }
    }
}
