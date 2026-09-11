use crate::components::chat::Chat;
use crate::components::sidebar::Sidebar;
use crate::components::ui::{Button, ButtonVariant, IconButton};
use crate::llm::LlmRuntimeConfig;
use crate::mock::{self, ChatMessage, MessagePart, Session, SessionStatus, TaskItem, ToolCall};
use dioxus::prelude::*;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub fn db_load_sessions(
    data_dir: &str,
) -> (Vec<Session>, HashMap<String, Vec<ChatMessage>>, String) {
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
            let last_ms = s.updated_at_ms as u64;
            sessions.push(Session {
                id: s.id.clone(),
                title: s.title.clone(),
                last_active: crate::mock::format_relative_time(last_ms),
                model: "default".into(),
                status: SessionStatus::Idle,
                last_active_epoch: last_ms,
            });
            if let Ok(db_msgs) = db.load_messages(&s.id, 100) {
                let chat_msgs: Vec<ChatMessage> = db_msgs
                    .into_iter()
                    .map(|m| {
                        let role = match m.role {
                            session::SessionRole::User => "user",
                            _ => "assistant",
                        };
                        let ts_rel = crate::mock::format_relative_time(m.created_at_ms as u64);
                        if let Ok(mut parsed) = serde_json::from_str::<ChatMessage>(&m.text) {
                            parsed.id = format!("{}-{}-{}", s.id, parsed.id, m.seq);
                            parsed.timestamp = ts_rel.clone();
                            parsed.ts_epoch_ms = m.created_at_ms as u64;
                            parsed
                        } else {
                            ChatMessage {
                                id: format!("{}-{}", s.id, m.seq),
                                role: role.into(),
                                content: m.text,
                                tool_calls: vec![],
                                parts: vec![],
                                timestamp: ts_rel,
                                ts_epoch_ms: m.created_at_ms as u64,
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
    // ponytail: 空库不要 mock 假会话，否则所有工作区显示同一批假数据
    (Vec::new(), HashMap::new(), String::new())
}

pub fn db_save_message(
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
pub fn read_subdirectories(parent: &Path) -> Vec<String> {
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

/// 从 `git worktree list --porcelain` 输出解析出 worktree 列表(去重)
pub fn parse_worktree_porcelain(
    porcelain: &str,
    active_path: &str,
) -> Vec<crate::components::sidebar::WorkspaceSpace> {
    use crate::components::sidebar::WorkspaceSpace;
    let mut list: Vec<WorkspaceSpace> = Vec::new();
    let mut cur_path = String::new();
    let mut cur_branch = String::new();
    let push_space = |path: &str, branch: &str, list: &mut Vec<WorkspaceSpace>| {
        if path.is_empty() {
            return;
        }
        if list.iter().any(|s| s.path == path) {
            return;
        }
        let name = std::path::Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string());
        let name_display = if branch == "main" {
            format!("{name} (main)")
        } else {
            name
        };
        list.push(WorkspaceSpace {
            id: path.to_string(),
            name: name_display,
            path: path.to_string(),
            branch: if branch.is_empty() {
                "detached".into()
            } else {
                branch.to_string()
            },
            is_active: path == active_path,
        });
    };
    for line in porcelain.lines() {
        if let Some(rest) = line.strip_prefix("worktree ") {
            cur_path = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("branch refs/heads/") {
            cur_branch = rest.trim().to_string();
        } else if line.is_empty() && !cur_path.is_empty() {
            push_space(&cur_path, &cur_branch, &mut list);
            cur_path.clear();
            cur_branch.clear();
        }
    }
    push_space(&cur_path, &cur_branch, &mut list);
    list
}

pub fn discover_workspaces(active_path: &str) -> Vec<crate::components::sidebar::WorkspaceSpace> {
    use crate::components::sidebar::WorkspaceSpace;
    let mut list: Vec<WorkspaceSpace> = Vec::new();

    if let Ok(output) = std::process::Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .output()
    {
        if output.status.success() {
            if let Ok(text) = String::from_utf8(output.stdout) {
                list = parse_worktree_porcelain(&text, active_path);
            }
        }
    }

    if list.is_empty() {
        list.push(WorkspaceSpace {
            id: active_path.to_string(),
            name: std::path::Path::new(active_path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "workspace".to_string()),
            path: active_path.to_string(),
            branch: "current".into(),
            is_active: true,
        });
    }

    list
}

pub fn load_tasks(data_dir: &str) -> Vec<TaskItem> {
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

    let initial_data = use_signal(|| db_load_sessions(&format!("{}/.oi", active_space_path())));
    // Full session list per worktree, so the sidebar accordion can show each
    // space's own sessions. Loaded once (off the LiveView task, via a thread);
    // kept in sync in the mutation handlers below.
    let mut space_sessions = use_signal(|| {
        let paths: Vec<String> = spaces().iter().map(|s| s.path.clone()).collect();
        std::thread::spawn(move || {
            let mut map: std::collections::HashMap<String, Vec<Session>> =
                std::collections::HashMap::new();
            for path in paths {
                let (sess, _, _) = db_load_sessions(&format!("{}/.oi", path));
                map.insert(path, sess);
            }
            map
        })
        .join()
        .unwrap_or_default()
    });
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

    let mut is_streaming = use_signal(|| false);
    let mut search_query = use_signal(|| String::new());
    // 侧边栏收起/展开 + 宽度：用全局信号，切换标签页(组件重建)后仍保持折叠/宽度
    let mut sidebar_collapsed = GlobalSignal::<bool>::new(|| false).signal();
    let mut sidebar_width = GlobalSignal::<usize>::new(|| 260).signal();
    // 拖拽边界（右边缘）：mousedown 记录起点，mousemove 更新宽度，mouseup 结束
    let mut dragging = use_signal(|| false);
    let mut drag_start_x = use_signal(|| 0i32);
    let mut drag_start_width = use_signal(|| 260usize);
    let on_resize_start = move |x: i32| {
        dragging.set(true);
        drag_start_x.set(x);
        drag_start_width.set(sidebar_width());
    };
    let on_resize_move = move |e: MouseEvent| {
        if dragging() {
            let raw =
                drag_start_width() as i32 + (e.client_coordinates().x as i32 - drag_start_x());
            // 拖到太窄 → 进入折叠(窄栏)状态
            sidebar_collapsed.set(raw < 80);
            sidebar_width.set((raw.max(80) as usize).min(480));
        }
    };
    let on_resize_up = move |_e: MouseEvent| dragging.set(false);
    let on_toggle_sidebar = move |_| {
        sidebar_collapsed.set(!sidebar_collapsed());
    };
    // 展开按钮：恢复默认宽度并展开
    let on_expand_sidebar = move |_| {
        sidebar_width.set(260);
        sidebar_collapsed.set(false);
    };
    // 折叠态右边缘预设把手：拖拽设定「默认展开宽度」并立即展开（宽度在应用运行期间持续维护）
    let mut preset_dragging = use_signal(|| false);
    let mut preset_drag_start_width = use_signal(|| 260usize);
    let on_preset_start = move |x: i32| {
        preset_dragging.set(true);
        preset_drag_start_width.set(sidebar_width());
    };
    let on_preset_move = move |e: MouseEvent| {
        if preset_dragging() {
            let target = 36_i32 + e.client_coordinates().x as i32;
            sidebar_width.set((target.max(220)).min(480) as usize);
            sidebar_collapsed.set(false);
        }
    };
    let on_preset_up = move |_e: MouseEvent| preset_dragging.set(false);
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
        sessions.set(new_sessions.clone());
        space_sessions.write().insert(path.clone(), new_sessions);
        session_messages.set(new_msgs);
        active_session_id.set(first_id);
    };
    let on_open_space = move |path: String| {
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
        sessions.set(new_sessions.clone());
        space_sessions.write().insert(path.clone(), new_sessions);
        session_messages.set(new_msgs);
        active_session_id.set(first_id);
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
        {
            // 若切到无会话的工作区+直接发送,先自建新会话再走原流程
            if active_session_id().is_empty()
                || !sessions().iter().any(|s| s.id == active_session_id())
            {
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis();
                let new_id = format!("s-{}", ts);
                let new_session = Session {
                    id: new_id.clone(),
                    title: format!("会话 {}", ts % 1_000_000),
                    last_active: "刚刚".into(),
                    model: config_send.model.clone(),
                    status: SessionStatus::Idle,
                    last_active_epoch: ts as u64,
                };
                let mut list = sessions();
                list.insert(0, new_session);
                sessions.set(list);
                space_sessions
                    .write()
                    .insert(active_space_path(), sessions().clone());
                let mut map = session_messages();
                map.insert(new_id.clone(), Vec::new());
                session_messages.set(map);
                active_session_id.set(new_id.clone());
                let data_dir = format!("{}/.oi", active_space_path());
                let sid2 = new_id.clone();
                let t = format!("会话 {}", ts % 1_000_000);
                std::thread::spawn(move || {
                    let path = std::path::PathBuf::from(data_dir).join("sessions.db");
                    if let Ok(db) = session::SessionDb::open(path) {
                        let _ = db.create_session(&sid2, &t);
                    }
                });
            }
        }
        {
            let mut list = sessions();
            if let Some(s) = list.iter_mut().find(|s| s.id == active_session_id()) {
                s.status = SessionStatus::Active;
            }
            sessions.set(list);
        }
        let sid = active_session_id();
        let existing_len = session_messages().get(&sid).map(|m| m.len()).unwrap_or(0);
        let user_id = format!("{}-user-{}", sid, existing_len + 1);
        let asst_id = format!("{}-asst-{}", sid, existing_len + 2);

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let user_msg = ChatMessage {
            id: user_id,
            role: "user".into(),
            content: text.clone(),
            tool_calls: vec![],
            parts: vec![],
            timestamp: "刚刚".into(),
            ts_epoch_ms: now_ms,
        };

        let asst_msg = ChatMessage {
            id: asst_id.clone(),
            role: "assistant".into(),
            content: String::new(),
            tool_calls: vec![],
            parts: vec![],
            timestamp: "刚刚".into(),
            ts_epoch_ms: now_ms,
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

        let data_dir_for_send = format!("{}/.oi", active_space_path());
        db_save_message(
            data_dir_for_send.clone(),
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
        let db_dir_for_events = data_dir_for_send.clone();

        spawn(async move {
            let mut total_out = 0u64;

            while let Some(ev) = rx.recv().await {
                match ev {
                    orbit::AgentEvent::AssistantText { delta } => {
                        total_out += 1;
                        let mut map = session_messages();
                        if let Some(list) = map.get_mut(&sid_for_events) {
                            if let Some(last) = list.last_mut() {
                                last.content.push_str(&delta);
                                // 保持发生顺序：追加到末尾文本段，否则新建一段
                                if let Some(MessagePart::Text(existing)) = last.parts.last_mut() {
                                    existing.push_str(&delta);
                                } else {
                                    last.parts.push(MessagePart::Text(delta.clone()));
                                }
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
                                last.tool_calls.push(tool_call.clone());
                                last.parts.push(MessagePart::Tool(tool_call));
                            }
                        }
                        session_messages.set(map);
                    }
                    orbit::AgentEvent::ToolResult {
                        id,
                        name: _,
                        result,
                    } => {
                        let is_err = result.starts_with("error") || result.contains("[exit ");
                        let new_status = if is_err {
                            "error".to_string()
                        } else {
                            "success".to_string()
                        };
                        let new_summary = if is_err {
                            "执行失败".to_string()
                        } else {
                            "执行完成".to_string()
                        };
                        let mut map = session_messages();
                        if let Some(list) = map.get_mut(&sid_for_events) {
                            if let Some(last) = list.last_mut() {
                                if let Some(target) =
                                    last.tool_calls.iter_mut().find(|t| t.id == id)
                                {
                                    target.status = new_status.clone();
                                    target.summary = new_summary.clone();
                                    target.detail = result.clone();
                                }
                                if let Some(MessagePart::Tool(target)) = last
                                    .parts
                                    .iter_mut()
                                    .find(|p| matches!(p, MessagePart::Tool(tc) if tc.id == id))
                                {
                                    target.status = new_status;
                                    target.summary = new_summary;
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
                                    last.parts.push(MessagePart::Text(last.content.clone()));
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

    let on_create_session = move |space_path: String| {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let new_id = format!("s-{}", ts);
        let title = format!("会话 {}", ts % 1_000_000);
        let new_session = Session {
            id: new_id.clone(),
            title: title.clone(),
            last_active: "刚刚".into(),
            model: config_create.model.clone(),
            status: SessionStatus::Idle,
            last_active_epoch: ts as u64,
        };
        let mut map = space_sessions.read().clone();
        let mut list = map.get(&space_path).cloned().unwrap_or_default();
        list.insert(0, new_session);
        map.insert(space_path.clone(), list.clone());
        space_sessions
            .write()
            .insert(space_path.clone(), list.clone());
        if space_path == active_space_path() {
            sessions.set(list);
        }
        // ponytail: 新会话不插欢迎气泡；空会话直挂对应工作区的 sessions.db
        let mut msgs = session_messages();
        msgs.insert(new_id.clone(), Vec::new());
        session_messages.set(msgs);
        if space_path == active_space_path() {
            active_session_id.set(new_id.clone());
        }

        let data_dir = format!("{}/.oi", space_path);
        let sid = new_id.clone();
        let title2 = title.clone();
        std::thread::spawn(move || {
            let path = std::path::PathBuf::from(data_dir).join("sessions.db");
            if let Ok(db) = session::SessionDb::open(path) {
                let _ = db.create_session(&sid, &title2);
            }
        });
    };

    let on_delete_session = move |id: String| {
        let mut map = space_sessions.read().clone();
        let mut target_path = active_space_path();
        for (p, list) in &map {
            if list.iter().any(|s| s.id == id) {
                target_path = p.clone();
                break;
            }
        }
        if let Some(list) = map.get_mut(&target_path) {
            list.retain(|s| s.id != id);
        }
        let new_list = map.get(&target_path).cloned().unwrap_or_default();
        space_sessions
            .write()
            .insert(target_path.clone(), new_list.clone());
        if target_path == active_space_path() {
            sessions.set(new_list.clone());
            if active_session_id() == id {
                if let Some(first) = new_list.first() {
                    active_session_id.set(first.id.clone());
                } else {
                    active_session_id.set(String::new());
                }
            }
        }
        let mut msgs = session_messages();
        msgs.remove(&id);
        session_messages.set(msgs);

        let data_dir = format!("{}/.oi", target_path);
        let del_id = id.clone();
        std::thread::spawn(move || {
            let path = std::path::PathBuf::from(data_dir).join("sessions.db");
            if let Ok(db) = session::SessionDb::open(path) {
                let _ = db.delete_session(&del_id);
            }
        });
    };

    let on_delete_space = move |space_path: String| {
        let mut sp = spaces();
        sp.retain(|s| s.path != space_path);
        spaces.set(sp.clone());
        space_sessions.write().remove(&space_path);
        if active_space_path() == space_path {
            if let Some(first) = sp.first() {
                let first_path = first.path.clone();
                active_space_path.set(first_path.clone());
                let list = space_sessions
                    .read()
                    .get(&first_path)
                    .cloned()
                    .unwrap_or_default();
                sessions.set(list.clone());
                if let Some(f) = list.first() {
                    active_session_id.set(f.id.clone());
                } else {
                    active_session_id.set(String::new());
                }
            } else {
                active_space_path.set(String::new());
                sessions.set(Vec::new());
                active_session_id.set(String::new());
            }
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
        div { class: "flex h-[calc(100vh-46px)] w-screen bg-base overflow-hidden relative",
            onmousemove: on_resize_move,
            onmouseup: on_resize_up,
            Sidebar {
                spaces: spaces(),
                on_select_space: on_select_space,
                on_trigger_picker: move |_| {
                    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/hathaway".to_string());
                    let p = format!("{}/projects", home);
                    picker_current_path.set(p.clone());
                    picker_subdirs.set(read_subdirectories(Path::new(&p)));
                    show_space_picker.set(true);
                },
                space_sessions: space_sessions.read().clone(),
                active_id: active_session_id(),
                on_select: on_select_session,
                on_create: on_create_session,
                on_delete_session: on_delete_session,
                on_delete_space: on_delete_space,
                collapsed: sidebar_collapsed(),
                on_toggle: on_toggle_sidebar,
                on_expand: on_expand_sidebar,
                width: sidebar_width(),
                on_resize_start: on_resize_start,
                on_preset_start: on_preset_start,
            }
            div { class: "flex-1 h-full flex flex-col bg-base relative overflow-hidden",
                div { class: "flex-1 flex flex-col h-full overflow-hidden",
                    div { class: "h-[44px] px-[24px] border-b border-subtle flex items-center gap-3 bg-base flex-shrink-0",
                        div { class: "flex items-center gap-2 text-[12px] text-muted-foreground",
                            span { class: "font-semibold text-foreground tracking-[-0.01em]", "omenic" }
                            span { class: "text-muted", "/" }
                            span { class: "font-mono text-[11.5px] text-muted-foreground", "feat/web-agent-harness" }
                            span { class: "font-mono text-[10.5px] px-1.5 py-0.5 rounded bg-accent-subtle text-accent border border-[rgba(162,138,199,0.2)]", "clean" }
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
                        class: "fixed top-0 left-0 w-screen h-screen bg-[rgba(0,0,0,0.75)] backdrop-blur-[6px] flex items-start justify-center pt-[14vh] z-[200]",
                        onclick: move |_| show_quick_switcher.set(false),
                        div {
                            class: "w-[560px] bg-surface border border-accent rounded-[10px] shadow-[0_24px_64px_rgba(0,0,0,0.9)] p-[12px] flex flex-col gap-[8px]",
                            onclick: move |_| {},
                            input {
                                class: "w-full py-[12px] px-[14px] bg-base border border-subtle rounded-[6px] text-foreground text-[13.5px] outline-none font-sans",
                                r#type: "text",
                                placeholder: "搜索会话名称或编号...",
                                oninput: move |e| search_query.set(e.value().clone()),
                                autofocus: true,
                            }
                            div { class: "max-h-[320px] overflow-y-auto flex flex-col gap-[2px]",
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
                                        class: if session.id == active_session_id() { "flex items-center justify-between px-[10px] py-[8px] rounded-[6px] cursor-pointer transition-colors bg-surface-elevated border border-accent" } else { "flex items-center justify-between px-[10px] py-[8px] rounded-[6px] cursor-pointer transition-colors hover:bg-hover border border-transparent" },
                                        onclick: {
                                            let id = session.id.clone();
                                            let mut on_select = on_select_session.clone();
                                            move |_| {
                                                on_select(id.clone());
                                                show_quick_switcher.set(false);
                                            }
                                        },
                                        div { class: "flex items-center gap-[8px]",
                                            span { class: if session.status == SessionStatus::Active { "inline-block w-2 h-2 rounded-full bg-accent shrink-0" } else { "inline-block w-2 h-2 rounded-full bg-muted shrink-0" } }
                                            span { class: "text-[12.5px] text-foreground", "{session.title}" }
                                        }
                                        span { class: "font-mono text-[10.5px] text-muted", "{session.id}" }
                                    }
                                }
                            }
                            div { class: "pt-[8px] border-t border-subtle flex items-center justify-between text-[11px] text-muted",
                                span { "选择会话快速切换" }
                                div { class: "flex items-center gap-1.5",
                                    kbd { "ESC" }
                                    span { "退出" }
                                }
                            }
                        }
                    }
                }
                if show_space_picker() {
                    div { class: "fixed top-0 left-0 w-screen h-screen bg-[rgba(0,0,0,0.75)] backdrop-blur-[6px] flex items-start justify-center pt-[14vh] z-[200]",
                        onclick: move |_| show_space_picker.set(false),
                        div { class: "w-[560px] bg-surface border border-accent rounded-[10px] shadow-[0_24px_64px_rgba(0,0,0,0.9)] p-[12px] flex flex-col gap-[8px]",
                            onclick: move |e: MouseEvent| e.stop_propagation(),
                            div { class: "flex items-center justify-between",
                                span { class: "text-[13px] font-semibold text-foreground", "选择本地工作区目录" }
                                IconButton {
                                    title: "关闭",
                                    onclick: move |_| show_space_picker.set(false),
                                    "✕"
                                }
                            }
                            div { class: "flex items-center gap-[8px]",
                                input {
                                    class: "flex-1 min-w-0 px-[12px] py-[9px] bg-base border border-subtle rounded-[6px] text-foreground text-[13px] outline-none font-mono",
                                    r#type: "text",
                                    value: "{picker_current_path()}",
                                    oninput: move |e| {
                                        let p = e.value();
                                        picker_current_path.set(p.clone());
                                        let sub = read_subdirectories(Path::new(&p));
                                        picker_subdirs.set(sub);
                                    }
                                }
                                Button {
                                    variant: ButtonVariant::Subtle,
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
                            div { class: "flex flex-col gap-[2px] overflow-y-auto max-h-[280px]",
                                if picker_subdirs().is_empty() {
                                    div { class: "p-3 text-[11.5px] text-muted", "（此路径下没有可见工作区子目录）" }
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
                                                class: "flex items-center justify-between py-[7px] px-[10px] rounded-[4px] cursor-pointer transition-all select-none",
                                                onclick: move |_| {
                                                    picker_current_path.set(dpath1.clone());
                                                    let sub = read_subdirectories(Path::new(&dpath1));
                                                    picker_subdirs.set(sub);
                                                },
                                                div { class: "flex items-center gap-[8px]",
                                                    span { class: "font-mono text-[9.5px] py-[1px] px-[5px] rounded-[3px] bg-[#20222e] text-muted", "DIR" }
                                                    span { class: "text-[12.5px] text-foreground", "{d}" }
                                                }
                                                Button {
                                                    variant: ButtonVariant::Subtle,
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
                            div { class: "flex items-center justify-between pt-[8px] border-t border-subtle text-[11px] text-muted",
                                div { class: "text-[11px] text-muted", span { "当前目录: {picker_current_path()}" } }
                                Button {
                                    variant: ButtonVariant::Primary,
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
