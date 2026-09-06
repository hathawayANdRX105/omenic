use crate::components::chat::Chat;
use crate::components::sidebar::Sidebar;
use crate::components::taskpanel::TaskPanel;
use crate::llm::LlmRuntimeConfig;
use crate::mock::{self, ChatMessage, Session, SessionStatus, TaskItem, ToolCall};
use dioxus::prelude::*;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

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
                        let role = match m.role {
                            session::SessionRole::User => "user",
                            _ => "assistant",
                        };
                        if let Ok(mut parsed) = serde_json::from_str::<ChatMessage>(&m.text) {
                            parsed.id = format!("{}-{}-{}", s.id, parsed.id, m.seq);
                            parsed
                        } else {
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
fn read_subdirectories(parent: &Path) -> Vec<String> {
    let mut dirs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            if let Ok(ft) = entry.file_type() {
                if ft.is_dir() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if !name.starts_with('.') && name != "node_modules" && name != "target" {
                        dirs.push(name);
                    }
                }
            }
        }
    }
    dirs.sort();
    dirs
}

fn discover_workspaces(active_path: &str) -> Vec<crate::components::sidebar::WorkspaceSpace> {
    use crate::components::sidebar::WorkspaceSpace;
    let mut list = Vec::new();

    if let Ok(output) = std::process::Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .output()
    {
        if output.status.success() {
            if let Ok(text) = String::from_utf8(output.stdout) {
                let mut cur_path = String::new();
                let mut cur_branch = String::new();
                for line in text.lines() {
                    if let Some(rest) = line.strip_prefix("worktree ") {
                        cur_path = rest.trim().to_string();
                    } else if let Some(rest) = line.strip_prefix("branch refs/heads/") {
                        cur_branch = rest.trim().to_string();
                    } else if line.is_empty() && !cur_path.is_empty() {
                        let name = std::path::Path::new(&cur_path)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| cur_path.clone());
                        let name_display = if cur_branch == "main" {
                            format!("{name} (main)")
                        } else {
                            name
                        };
                        let is_active = cur_path == active_path;
                        list.push(WorkspaceSpace {
                            id: cur_path.clone(),
                            name: name_display,
                            path: cur_path.clone(),
                            branch: if cur_branch.is_empty() {
                                "detached".into()
                            } else {
                                cur_branch.clone()
                            },
                            is_active,
                        });
                        cur_path.clear();
                        cur_branch.clear();
                    }
                }
                if !cur_path.is_empty() {
                    let name = std::path::Path::new(&cur_path)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| cur_path.clone());
                    let is_active = cur_path == active_path;
                    list.push(WorkspaceSpace {
                        id: cur_path.clone(),
                        name,
                        path: cur_path.clone(),
                        branch: cur_branch,
                        is_active,
                    });
                }
            }
        }
    }

    if list.is_empty() {
        list.push(WorkspaceSpace {
            id: active_path.to_string(),
            name: "web-agent-harness".to_string(),
            path: active_path.to_string(),
            branch: "feat/web-agent-harness".to_string(),
            is_active: true,
        });
    }

    list
}

fn load_tasks(data_dir: &str) -> Vec<TaskItem> {
    let store = task::store::Store::new(Path::new(data_dir));
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
    show_quick_switcher: Signal<bool>,
) -> Element {
    let mut active_space_path = use_signal(|| {
        std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| ".".to_string())
    });
    let mut spaces = use_signal(|| discover_workspaces(&active_space_path()));

    let initial_data = use_signal(|| db_load_sessions(&config.data_dir));
    let mut sessions = use_signal(|| {
        let mut list = initial_data.read().0.clone();
        let target_id = initial_data.read().2.clone();
        if let Some(s) = list.iter_mut().find(|s| s.id == target_id) {
            s.status = SessionStatus::Idle;
        }
        list
    });
    let mut active_session_id = use_signal(|| initial_data.read().2.clone());
    let mut session_messages = use_signal(|| initial_data.read().1.clone());

    let mut statusline = use_signal(|| {
        let mut st = mock::mock_statusline();
        st.model = config.model.clone();
        st.cwd = config.data_dir.clone();
        st.tokens_in = 0;
        st.tokens_out = 0;
        st.cost_usd = 0.0;
        st.context_pct = 0.0;
        st
    });

    let mut tasks = use_signal(|| load_tasks(&config.data_dir));
    let mut show_tasks = use_signal(|| true);
    let mut is_streaming = use_signal(|| false);
    let mut search_query = use_signal(|| String::new());
    // Space Directory Picker state
    let mut show_space_picker = use_signal(|| false);
    let mut picker_current_path = use_signal(|| {
        let home = std::env::var("HOME").expect("HOME not set");
        format!("{}/projects", home)
    });
    let mut picker_subdirs = use_signal(|| {
        let home = std::env::var("HOME").expect("HOME not set");
        read_subdirectories(Path::new(&format!("{}/projects", home)))
    });

    let on_select_space = move |path: String| {
        active_space_path.set(path.clone());
        let oi_dir = format!("{path}/.oi");
        let (new_sessions, new_msgs, first_id) = db_load_sessions(&oi_dir);
        sessions.set(new_sessions);
        session_messages.set(new_msgs);
        active_session_id.set(first_id);
        let fresh_tasks = load_tasks(&oi_dir);
        tasks.set(fresh_tasks);
    };
    let mut on_open_space = move |path: String| {
        let name = std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        let new_space = crate::components::sidebar::WorkspaceSpace {
            id: path.clone(),
            name,
            path: path.clone(),
            branch: "custom".into(),
            is_active: true,
        };
        let mut list = spaces();
        list.insert(0, new_space);
        spaces.set(list);
        active_space_path.set(path.clone());
        let oi_dir = format!("{path}/.oi");
        let (new_sessions, new_msgs, first_id) = db_load_sessions(&oi_dir);
        sessions.set(new_sessions);
        session_messages.set(new_msgs);
        active_session_id.set(first_id);
        let fresh_tasks = load_tasks(&oi_dir);
        tasks.set(fresh_tasks);
    };

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

    let on_send = move |text: String| {
        let sid = active_session_id();
        {
            let mut list = sessions();
            if let Some(s) = list.iter_mut().find(|s| s.id == sid) {
                s.status = SessionStatus::Active;
            }
            sessions.set(list);
        }
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

        db_save_message(
            config_send.data_dir.clone(),
            sid.clone(),
            Some(active_title_for_send.clone()),
            session::SessionRole::User,
            text.clone(),
        );

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
        let abort_signal = Arc::new(AtomicBool::new(false));

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
            st.tokens_out += total_out;
            st.tokens_in = (history.iter().map(|m| m.content.len()).sum::<usize>() / 4) as u64;
            st.cost_usd += total_out as f64 * 0.000002;
            st.context_pct =
                ((st.tokens_in + st.tokens_out) as f64 / st.context_max as f64 * 100.0).min(100.0);
            statusline.set(st);

            let sid_for_cleanup = sid_for_events.clone();
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

            let mut list = sessions();
            if let Some(s) = list.iter_mut().find(|s| s.id == sid_for_cleanup) {
                s.status = SessionStatus::Idle;
            }
            sessions.set(list);
        });
    };

    let on_select_session = move |id: String| {
        let mut list = sessions();
        for s in list.iter_mut() {
            if s.id == id {
                s.status = if s.status == SessionStatus::Active {
                    SessionStatus::Active
                } else {
                    SessionStatus::Idle
                };
            }
        }
        sessions.set(list);
        active_session_id.set(id);
    };

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

    let on_model_change = move |m: String| {
        let mut st = statusline();
        st.model = m.clone();
        statusline.set(st);

        let mut cfg = config_model.clone();
        cfg.model = m;
        on_update_config.call(cfg);
    };

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
                spaces: spaces(),
                active_space_id: active_space_path(),
                on_select_space: on_select_space,
                on_trigger_picker: move |_| {
                    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/hathaway".to_string());
                    let p = format!("{}/projects", home);
                    picker_current_path.set(p.clone());
                    picker_subdirs.set(read_subdirectories(Path::new(&p)));
                    show_space_picker.set(true);
                },
                sessions: sessions(),
                active_id: active_session_id(),
                on_select: on_select_session,
                on_create: on_create_session,
            }
            div { class: "workspace-main",
                div { class: "workspace-chat-area",
                    div { class: "chat-header-bar",
                        div { class: "chat-header-left",
                            span { class: "chat-header-title", "{active_title}" }
                            span { class: "chat-header-badge", "{config.model}" }
                        }
                        button {
                            class: if show_tasks() { "btn-task-toggle active" } else { "btn-task-toggle" },
                            onclick: move |_| show_tasks.set(!show_tasks()),
                            span { "任务看板 {tasks.len()} ▾" }
                        }
                    }
                    Chat {
                        messages: current_messages,
                        statusline: statusline(),
                        is_streaming: is_streaming(),
                        on_send: on_send,
                        on_model_change: on_model_change,
                        on_toggle_thinking: on_toggle_thinking,
                    }
                }
                if show_quick_switcher() {
                    div {
                        class: "modal-backdrop",
                        onclick: move |_| show_quick_switcher.set(false),
                        div {
                            class: "quick-switcher-modal",
                            onclick: move |_| {},
                            input {
                                class: "switcher-input",
                                r#type: "text",
                                placeholder: "搜索会话名称或编号...",
                                value: "{search_query}",
                                oninput: move |e| search_query.set(e.value().clone()),
                                autofocus: true,
                            }
                            div { class: "switcher-results",
                                for session in sessions()
                                    .iter()
                                    .filter(|s| {
                                        let query = search_query();
                                        let q = query.to_lowercase();
                                        query.is_empty() || s.title.to_lowercase().contains(&q) || s.id.to_lowercase().contains(&q)
                                    })
                                {
                                    div {
                                        key: "{session.id}",
                                        class: if session.id == active_session_id() { "switcher-item selected" } else { "switcher-item" },
                                        onclick: {
                                            let id = session.id.clone();
                                            let mut on_select = on_select_session.clone();
                                            move |_| {
                                                on_select(id.clone());
                                                show_quick_switcher.set(false);
                                            }
                                        },
                                        div { class: "switcher-item-left",
                                            span { class: if session.status == SessionStatus::Active { "status-dot run" } else { "status-dot idle" } }
                                            span { class: "switcher-title", "{session.title}" }
                                        }
                                        span { class: "switcher-id", "{session.id}" }
                                    }
                                }
                            }
                            div { class: "switcher-footer",
                                span { "选择会话快速切换" }
                                div { style: "display: flex; gap: 6px; align-items: center;",
                                    kbd { "ESC" }
                                    span { "退出" }
                                }
                            }
                        }
                    }
                }
                if show_tasks() {
                    TaskPanel {
                        tasks: tasks(),
                        on_close: move |_| show_tasks.set(false),
                    }
                }
                if show_space_picker() {
                    div { class: "modal-backdrop",
                        onclick: move |_| show_space_picker.set(false),
                        div { class: "space-picker-modal",
                            onclick: move |e: MouseEvent| e.stop_propagation(),
                            div { class: "space-picker-header",
                                span { class: "space-picker-title", "选择本地工作区目录" }
                                button {
                                    class: "floating-task-close",
                                    onclick: move |_| show_space_picker.set(false),
                                    "✕"
                                }
                            }
                            div { class: "space-picker-path-row",
                                input {
                                    class: "space-picker-path-input",
                                    r#type: "text",
                                    value: "{picker_current_path()}",
                                    oninput: move |e| {
                                        let p = e.value();
                                        picker_current_path.set(p.clone());
                                        let sub = read_subdirectories(Path::new(&p));
                                        picker_subdirs.set(sub);
                                    }
                                }
                                button {
                                    class: "btn-picker-nav",
                                    onclick: move |_| {
                                        let par = Path::new(&picker_current_path()).parent().map(|sp| sp.to_path_buf());
                                        if let Some(pr) = par {
                                            let s = pr.display().to_string();
                                            picker_current_path.set(s.clone());
                                            let sub = read_subdirectories(&pr);
                                            picker_subdirs.set(sub);
                                        }
                                    },
                                    ".. 上级目录"
                                }
                            }
                            div { class: "space-picker-folder-list",
                                if picker_subdirs().is_empty() {
                                    div { style: "padding: 12px; color: var(--text-muted); font-size: 11.5px;", "（此路径下没有可见工作区子目录）" }
                                }
                                for dir_name in picker_subdirs() {
                                    {
                                        let d = dir_name.clone();
                                        let dpath1 = format!("{}/{}", picker_current_path(), d);
                                        let dpath2 = dpath1.clone();
                                        let mut on_open_fn = on_open_space.clone();
                                        rsx! {
                                            div {
                                                key: "{dir_name}",
                                                class: "folder-item-row",
                                                onclick: move |_| {
                                                    picker_current_path.set(dpath1.clone());
                                                    let sub = read_subdirectories(Path::new(&dpath1));
                                                    picker_subdirs.set(sub);
                                                },
                                                div { class: "folder-item-left",
                                                    span { class: "folder-badge", "DIR" }
                                                    span { class: "folder-name", "{d}" }
                                                }
                                                button {
                                                    class: "btn-picker-nav",
                                                    onclick: move |e: MouseEvent| {
                                                        e.stop_propagation();
                                                        on_open_fn(dpath2.clone());
                                                        show_space_picker.set(false);
                                                    },
                                                    "打开"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            div { class: "space-picker-footer",
                                div { class: "space-picker-footer-info", span { "当前目录: {picker_current_path()}" } }
                                button {
                                    class: "btn-confirm-space",
                                    onclick: {
                                        let mut on_open_fn = on_open_space.clone();
                                        move |_| {
                                            on_open_fn(picker_current_path());
                                            show_space_picker.set(false);
                                        }
                                    },
                                    "选择此目录"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
