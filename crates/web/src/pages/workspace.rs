use crate::components::chat::Chat;
use crate::components::sidebar::Sidebar;
use crate::components::taskpanel::TaskPanel;
use crate::llm::LlmRuntimeConfig;
use crate::mock::{self, ChatMessage, Session, SessionStatus, TaskItem, ToolCall};
use dioxus::prelude::*;
use std::collections::HashMap;

fn db_load_sessions(data_dir: &str) -> (Vec<Session>, HashMap<String, Vec<ChatMessage>>, String) {
    let path = std::path::PathBuf::from(data_dir).join("sessions.db");
    let res = std::thread::spawn(move || -> Result<_, session::SessionError> {
        let db = session::SessionDb::open(path)?;
        let summaries = db.list_sessions("%", 50)?;
        if summaries.is_empty() {
            return Ok(None);
        }
        let mut sessions = Vec::new();
        let mut messages = HashMap::new();
        let first_id = summaries[0].id.clone();
        for s in summaries {
            sessions.push(Session {
                id: s.id.clone(),
                title: s.title.clone(),
                last_active: "刚刚".into(),
                model: "default".into(),
                status: SessionStatus::Active,
            });
            if let Ok(db_msgs) = db.load_messages(&s.id, 100) {
                let chat_msgs: Vec<ChatMessage> = db_msgs
                    .into_iter()
                    .map(|m| {
                        if let Ok(parsed) = serde_json::from_str::<ChatMessage>(&m.text) {
                            parsed
                        } else {
                            let role = match m.role {
                                session::SessionRole::User => "user",
                                _ => "assistant",
                            };
                            ChatMessage {
                                id: format!("{}-{}", s.id, m.seq),
                                role: role.into(),
                                content: m.text,
                                tool_calls: vec![],
                                timestamp: "历史".into(),
                            }
                        }
                    })
                    .collect();
                messages.insert(s.id, chat_msgs);
            }
        }
        Ok(Some((sessions, messages, first_id)))
    })
    .join();

    if let Ok(Ok(Some(data))) = res {
        return data;
    }

    let mut default_map = HashMap::new();
    default_map.insert("s1".into(), mock::mock_messages_for_session("s1"));
    default_map.insert("s2".into(), mock::mock_messages_for_session("s2"));
    default_map.insert("s3".into(), mock::mock_messages_for_session("s3"));
    (mock::mock_sessions(), default_map, "s1".to_string())
}

fn db_save_message(
    data_dir: String,
    sid: String,
    title: Option<String>,
    role: session::SessionRole,
    text: String,
) {
    std::thread::spawn(move || {
        let path = std::path::PathBuf::from(data_dir).join("sessions.db");
        if let Ok(db) = session::SessionDb::open(path) {
            if let Some(t) = title {
                let _ = db.create_session(&sid, &t);
            }
            let _ = db.append_message(&sid, role, &text);
        }
    });
}
fn load_tasks(data_dir: &str) -> Vec<TaskItem> {
    let store = task::store::Store::new(std::path::Path::new(data_dir));
    if let Ok(real_tasks) = store.load_all() {
        if !real_tasks.is_empty() {
            return real_tasks
                .into_iter()
                .map(|t| {
                    let status = match t.status {
                        task::TaskStatus::Open => "open".to_string(),
                        task::TaskStatus::InProgress => "in_progress".to_string(),
                        task::TaskStatus::Failed => "failed".to_string(),
                        task::TaskStatus::Done => "done".to_string(),
                    };
                    TaskItem {
                        id: t.id,
                        title: t.title,
                        status,
                        kind: format!("{:?}", t.kind).to_lowercase(),
                        priority: t.priority,
                        description: t.description,
                        acceptance: t.acceptance,
                    }
                })
                .collect();
        }
    }
    mock::mock_tasks()
}

#[component]
pub fn Workspace(
    config: LlmRuntimeConfig,
    on_update_config: EventHandler<LlmRuntimeConfig>,
) -> Element {
    let initial_data = use_signal(|| db_load_sessions(&config.data_dir));
    let mut sessions = use_signal(|| initial_data.read().0.clone());
    let mut active_session_id = use_signal(|| initial_data.read().2.clone());
    let mut session_messages = use_signal(|| initial_data.read().1.clone());

    let mut statusline = use_signal(|| {
        let mut st = mock::mock_statusline();
        st.model = config.model.clone();
        st
    });

    let tasks = load_tasks(&config.data_dir);
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
        .unwrap_or_default();

    let config_send = config.clone();
    let config_create = config.clone();
    let config_model = config.clone();
    let active_title_for_send = active_title.clone();

    // Handler: Send Message with Real Orbit Agent Loop
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

        let asst_msg = ChatMessage {
            id: asst_id.clone(),
            role: "assistant".into(),
            content: String::new(),
            tool_calls: vec![],
            timestamp: "刚刚".into(),
        };

        let history = {
            let mut map = session_messages();
            let list = map.entry(sid.clone()).or_insert_with(Vec::new);
            list.push(user_msg.clone());
            list.push(asst_msg);
            let h = list.clone();
            session_messages.set(map);
            h
        };

        is_streaming.set(true);

        // 1. Persist user message to SessionDb
        db_save_message(
            config_send.data_dir.clone(),
            sid.clone(),
            Some(active_title_for_send.clone()),
            session::SessionRole::User,
            text.clone(),
        );
        // 2. Prepare Context for orbit::run_agent_streaming
        let mut context = adaptor::Context::default();
        for m in history.iter().take(history.len().saturating_sub(1)) {
            if m.role == "user" {
                context
                    .messages
                    .push(adaptor::Message::user_text(&m.content));
            } else if m.role == "assistant" && !m.content.is_empty() {
                context
                    .messages
                    .push(adaptor::Message::assistant_text(&m.content));
            }
        }

        let clean_base = config_send.base_url.trim_end_matches('/');
        let base_url = if clean_base.ends_with("/v1") {
            clean_base.to_string()
        } else {
            format!("{}/v1", clean_base)
        };
        let model = adaptor::Model {
            api_key: config_send.api_key.clone(),
            model: config_send.model.clone(),
            base_url: Some(base_url),
            max_tokens: Some(config_send.max_tokens),
        };

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<orbit::AgentEvent>();
        let abort_signal = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        std::thread::spawn(move || {
            let backend = orbit::HttpLlm;
            let tools = tools::builtin_tools();
            orbit::run_agent_streaming(
                &backend,
                &model,
                &mut context,
                &tools,
                &abort_signal,
                None,
                &mut |ev| {
                    let _ = tx.send(ev);
                },
            );
        });

        let sid_for_events = sid.clone();
        let db_dir_for_events = config_send.data_dir.clone();

        spawn(async move {
            let total_in = 0u64;
            let mut total_out = 0u64;

            while let Some(ev) = rx.recv().await {
                match ev {
                    orbit::AgentEvent::AssistantText { delta } => {
                        total_out += 1;
                        let mut map = session_messages();
                        if let Some(list) = map.get_mut(&sid_for_events) {
                            if let Some(last) = list.last_mut() {
                                last.content.push_str(&delta);
                            }
                        }
                        session_messages.set(map);
                    }
                    orbit::AgentEvent::ToolCall(tc) => {
                        let kind = match tc.name.as_str() {
                            "run_bash" => "bash",
                            "edit" => "edit",
                            "read_file" => "read",
                            "write_file" => "write",
                            "delete_file" => "delete",
                            "grep" => "grep",
                            "glob" => "glob",
                            _ => tc.name.as_str(),
                        }
                        .to_string();

                        let title = match tc.name.as_str() {
                            "run_bash" => tc
                                .args
                                .get("command")
                                .and_then(|v| v.as_str())
                                .unwrap_or(&tc.name)
                                .to_string(),
                            "edit" | "read_file" | "write_file" | "delete_file" => tc
                                .args
                                .get("path")
                                .and_then(|v| v.as_str())
                                .unwrap_or(&tc.name)
                                .to_string(),
                            _ => tc.name.clone(),
                        };

                        let tool_call = ToolCall {
                            id: tc.id.clone(),
                            title,
                            kind,
                            summary: "正在执行...".to_string(),
                            detail: serde_json::to_string_pretty(&tc.args).unwrap_or_default(),
                            status: "running".to_string(),
                        };

                        let mut map = session_messages();
                        if let Some(list) = map.get_mut(&sid_for_events) {
                            if let Some(last) = list.last_mut() {
                                last.tool_calls.push(tool_call);
                            }
                        }
                        session_messages.set(map);
                    }
                    orbit::AgentEvent::ToolResult {
                        id,
                        name: _,
                        result,
                    } => {
                        let mut map = session_messages();
                        if let Some(list) = map.get_mut(&sid_for_events) {
                            if let Some(last) = list.last_mut() {
                                if let Some(target) =
                                    last.tool_calls.iter_mut().find(|t| t.id == id)
                                {
                                    let is_err =
                                        result.starts_with("error") || result.contains("[exit ");
                                    target.status = if is_err {
                                        "error".to_string()
                                    } else {
                                        "success".to_string()
                                    };
                                    target.summary = if is_err {
                                        "执行失败".to_string()
                                    } else {
                                        "执行完成".to_string()
                                    };
                                    target.detail = result;
                                }
                            }
                        }
                        session_messages.set(map);
                    }
                    orbit::AgentEvent::TurnEnd { stop_reason } => {
                        let mut map = session_messages();
                        if let Some(list) = map.get_mut(&sid_for_events) {
                            if let Some(last) = list.last_mut() {
                                if last.content.is_empty() && last.tool_calls.is_empty() {
                                    last.content = format!(
                                        "Agent 执行结束（原因: {:?}）。未能获取有效回复，请在「配置」页检查 API 凭证与端点地址。",
                                        stop_reason
                                    );
                                }
                            }
                        }
                        session_messages.set(map);
                    }
                }
            }

            let mut st = statusline();
            st.tokens_in += total_in + 80;
            st.tokens_out += total_out;
            st.cost_usd += (total_in + total_out) as f64 * 0.000002;
            statusline.set(st);

            // Persist completed assistant message with its tool_calls
            let final_asst_msg = session_messages()
                .get(&sid_for_events)
                .and_then(|list| list.last().cloned());
            if let Some(final_msg) = final_asst_msg {
                if let Ok(serialized) = serde_json::to_string(&final_msg) {
                    db_save_message(
                        db_dir_for_events,
                        sid_for_events,
                        None,
                        session::SessionRole::Assistant,
                        serialized,
                    );
                }
            }

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
        let title = format!("任务 #{}", new_num);
        let new_session = Session {
            id: new_id.clone(),
            title: title.clone(),
            last_active: "刚刚".into(),
            model: config_create.model.clone(),
            status: SessionStatus::Active,
        };
        let mut current_sessions = sessions();
        current_sessions.insert(0, new_session);
        sessions.set(current_sessions);

        let welcome_msg = ChatMessage {
            id: format!("{}-welcome", new_id),
            role: "assistant".into(),
            content: format!(
                "新任务已创建。当前配置模型为 `{}` (端点 `{}`)，支持使用 bash、文件读写与代码编辑工具，输入指令开始执行。",
                config_create.model, config_create.base_url
            ),
            tool_calls: vec![],
            timestamp: "刚刚".into(),
        };

        let mut map = session_messages();
        map.insert(new_id.clone(), vec![welcome_msg.clone()]);
        session_messages.set(map);
        active_session_id.set(new_id.clone());

        if let Ok(ser) = serde_json::to_string(&welcome_msg) {
            db_save_message(
                config_create.data_dir.clone(),
                new_id.clone(),
                Some(title),
                session::SessionRole::Assistant,
                ser,
            );
        }
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
                    // Header Bar (zcode style)
                    div { class: "chat-header-bar",
                        div { class: "chat-header-title",
                            span { "{active_title}" }
                            span { class: "chat-header-badge", "{config.model}" }
                        }
                        button {
                            class: if show_tasks() { "btn-toggle-taskpanel active" } else { "btn-toggle-taskpanel" },
                            onclick: move |_| show_tasks.set(!show_tasks()),
                            if show_tasks() { "隐藏看板" } else { "任务看板" }
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

                // Right Panel (zcode Tasks Panel)
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
