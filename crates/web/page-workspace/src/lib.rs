//! 工作区页：dsh AppFrame 三列布局（侧栏 280 / 中栏）。
//!
//! 数据源 = `omenic-web-mock`（假数据 + AgentEvent 模拟流）；G4 时把
//! `mock::stream_reply` 换成 daemon `event.subscribe` 接收端即可，页面零改动。

use std::collections::HashMap;

use dioxus::prelude::*;
use omenic_web_client::llm::LlmRuntimeConfig;
use omenic_web_components::chat::Chat;
use omenic_web_components::sidebar::Sidebar;
use omenic_web_components::taskpanel::TaskPanel;
use omenic_web_components::ui::Modal;
use omenic_web_mock::store;
use omenic_web_page_config::SettingsModal;
use omenic_web_page_stats::StatsView;
use omenic_web_state::types::{
    ChatMessage, Session, SessionStatus, WorkspaceSpace, format_relative_time,
};
use omenic_web_state::ui_state::{AgentEvent, UiState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Chat,
    Stats,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// 新建会话（mock 版；G4 后替换为 RPC 调用）。独立成自由函数：
/// Signal 是 Copy，任意闭包都可以直接调用，避免处理器闭包被多处 move。
#[allow(clippy::too_many_arguments)]
fn create_session_in(
    space_path: String,
    model: String,
    mut space_sessions: Signal<HashMap<String, Vec<Session>>>,
    mut session_messages: Signal<HashMap<String, Vec<ChatMessage>>>,
    mut active_space_path: Signal<String>,
    mut active_session_id: Signal<String>,
) -> String {
    let ts = now_ms();
    let new_id = format!("s-{}", ts);
    let new_session = Session {
        id: new_id.clone(),
        title: format!("会话 {}", ts % 1_000_000),
        last_active: "刚刚".into(),
        model,
        status: SessionStatus::Idle,
        last_active_epoch: ts,
    };
    let mut map = space_sessions.read().clone();
    let mut list = map.get(&space_path).cloned().unwrap_or_default();
    list.insert(0, new_session);
    map.insert(space_path.clone(), list);
    space_sessions.set(map);
    session_messages.write().insert(new_id.clone(), Vec::new());
    active_space_path.set(space_path);
    active_session_id.set(new_id.clone());
    new_id
}

/// 会话是否仍存在于任一 space。流式期间会话可能被用户删除
/// （`on_delete_session` 会同时清掉消息），孤儿 entry 不能写回。
fn session_exists_in(space_sessions: Signal<HashMap<String, Vec<Session>>>, sid: &str) -> bool {
    space_sessions
        .read()
        .values()
        .flatten()
        .any(|s| s.id == sid)
}

#[component]
pub fn Workspace(
    config: LlmRuntimeConfig,
    on_update_config: EventHandler<LlmRuntimeConfig>,
) -> Element {
    let mut spaces = use_signal(store::spaces);
    let mut active_space_path = use_signal(|| {
        spaces
            .read()
            .iter()
            .find(|s| s.is_active)
            .map(|s| s.path.clone())
            .unwrap_or_default()
    });
    let mut space_sessions: Signal<HashMap<String, Vec<Session>>> = use_signal(|| {
        let mut map = HashMap::new();
        for space in store::spaces() {
            map.insert(space.path.clone(), store::sessions_for_space(&space.path));
        }
        map
    });
    let mut active_session_id = use_signal(|| {
        space_sessions
            .read()
            .get(&active_space_path())
            .and_then(|list| list.first().map(|s| s.id.clone()))
            .unwrap_or_default()
    });
    let mut session_messages: Signal<HashMap<String, Vec<ChatMessage>>> = use_signal(HashMap::new);
    // 打开即有内容：预填当前会话的 fixture 消息
    use_effect(move || {
        let sid = active_session_id();
        if !sid.is_empty() && !session_messages.read().contains_key(&sid) {
            session_messages
                .write()
                .insert(sid.clone(), store::messages_for_session(&sid));
        }
    });

    let mut statusline = use_signal(|| {
        let mut st = store::statusline();
        st.model = config.model.clone();
        st
    });
    let mut is_streaming = use_signal(|| false);
    let mut view = use_signal(|| View::Chat);
    let mut show_quick_switcher = use_signal(|| false);
    let mut show_settings = use_signal(|| false);
    let mut show_tasks = use_signal(|| false);
    let mut search_query = use_signal(String::new);

    // 侧栏宽度/折叠：全局信号，切视图后仍保持
    let mut sidebar_collapsed = GlobalSignal::<bool>::new(|| false).signal();
    let mut sidebar_width = GlobalSignal::<usize>::new(|| 280).signal();
    let mut dragging = use_signal(|| false);
    let mut drag_start_x = use_signal(|| 0i32);
    let mut drag_start_width = use_signal(|| 280usize);
    let on_resize_start = move |x: i32| {
        dragging.set(true);
        drag_start_x.set(x);
        drag_start_width.set(sidebar_width());
    };
    let mut on_resize_move = move |e: MouseEvent| {
        // 自愈：拖拽中鼠标键已全部松开（mouseup 在窗口外被吞）→ 直接结束
        if e.held_buttons().is_empty() {
            dragging.set(false);
            return;
        }
        if dragging() {
            let raw =
                drag_start_width() as i32 + (e.client_coordinates().x as i32 - drag_start_x());
            sidebar_collapsed.set(raw < 100);
            sidebar_width.set((raw.max(100) as usize).min(420));
        }
    };
    let on_toggle_sidebar = move |_| sidebar_collapsed.set(!sidebar_collapsed());
    let on_expand_sidebar = move |_| {
        sidebar_width.set(280);
        sidebar_collapsed.set(false);
    };
    let mut preset_dragging = use_signal(|| false);
    let mut on_preset_move = move |e: MouseEvent| {
        if e.held_buttons().is_empty() {
            preset_dragging.set(false);
            return;
        }
        if preset_dragging() {
            let target = 56_i32 + e.client_coordinates().x as i32;
            sidebar_width.set(target.clamp(264, 420) as usize);
            sidebar_collapsed.set(false);
        }
    };
    let on_root_mousemove = move |e: MouseEvent| {
        on_resize_move(e.clone());
        on_preset_move(e);
    };
    let on_root_mouseup = move |_e: MouseEvent| {
        dragging.set(false);
        preset_dragging.set(false);
    };

    let active_space = spaces()
        .iter()
        .find(|s| s.path == active_space_path())
        .cloned()
        .unwrap_or_else(|| WorkspaceSpace {
            id: String::new(),
            name: "omenic".into(),
            path: String::new(),
            branch: String::new(),
            is_active: true,
        });
    let active_title = space_sessions
        .read()
        .values()
        .flatten()
        .find(|s| s.id == active_session_id())
        .map(|s| s.title.clone())
        .unwrap_or_else(|| "新会话".into());

    let current_messages = session_messages()
        .get(&active_session_id())
        .cloned()
        .unwrap_or_default();

    // 各 move 闭包各自的 config 克隆（LlmRuntimeConfig 非 Copy）
    let config_create = config.clone();
    let config_model = config.clone();
    let config_send = config.clone();

    // ── 数据操作（mock 版；G4 后替换为 RPC 调用）───────────────────────────

    let on_select_space = move |path: String| {
        active_space_path.set(path);
    };

    let on_create_session = move |space_path: String| {
        create_session_in(
            space_path,
            config_create.model.clone(),
            space_sessions,
            session_messages,
            active_space_path,
            active_session_id,
        );
        view.set(View::Chat);
    };

    let on_delete_session = move |id: String| {
        let mut map = space_sessions.read().clone();
        for list in map.values_mut() {
            list.retain(|s| s.id != id);
        }
        space_sessions.set(map);
        session_messages.write().remove(&id);
        if active_session_id() == id {
            let next = space_sessions
                .read()
                .get(&active_space_path())
                .and_then(|list| list.first().map(|s| s.id.clone()))
                .unwrap_or_default();
            active_session_id.set(next);
        }
    };

    let on_delete_space = move |space_path: String| {
        let mut sp = spaces();
        sp.retain(|s| s.path != space_path);
        spaces.set(sp);
        space_sessions.write().remove(&space_path);
        if active_space_path() == space_path {
            if let Some(first) = spaces().first() {
                active_space_path.set(first.path.clone());
            } else {
                active_space_path.set(String::new());
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
            "8k".into()
        } else {
            "off".into()
        };
        statusline.set(st);
    };

    // ── 发送：mock 模拟流 → 转译层 ─────────────────────────────────────────

    let on_send = move |text: String| {
        // 无会话时先建一个
        if active_session_id().is_empty()
            || !space_sessions
                .read()
                .values()
                .any(|list| list.iter().any(|s| s.id == active_session_id()))
        {
            create_session_in(
                active_space_path(),
                config_send.model.clone(),
                space_sessions,
                session_messages,
                active_space_path,
                active_session_id,
            );
        }
        let sid = active_session_id();

        let now = now_ms();
        let user_msg = ChatMessage {
            id: format!("{}-user-{}", sid, now),
            role: "user".into(),
            content: text.clone(),
            tool_calls: vec![],
            parts: vec![],
            timestamp: "刚刚".into(),
            ts_epoch_ms: now,
        };
        session_messages
            .write()
            .entry(sid.clone())
            .or_default()
            .push(user_msg);

        // 会话进入运行态
        let mut map = space_sessions.read().clone();
        for list in map.values_mut() {
            for s in list.iter_mut() {
                if s.id == sid {
                    s.status = SessionStatus::Active;
                    s.last_active = "刚刚".into();
                    s.last_active_epoch = now;
                }
            }
        }
        space_sessions.set(map);
        is_streaming.set(true);

        let mut rx = omenic_web_mock::stream::stream_reply(text);
        spawn(async move {
            let mut total_out: u64 = 0;
            // 读取与写回在同一把写锁内完成，避免跨锁的读后写窗口
            while let Some(ev) = rx.recv().await {
                if matches!(ev, AgentEvent::TurnEnd { .. }) {
                    break;
                }
                if matches!(ev, AgentEvent::AssistantText { .. }) {
                    total_out += 1;
                }
                // 流式期间会话可能已被删除：事件照常消费，但消息不写回，
                // 避免把已删会话的孤儿 entry 重新写进 session_messages
                if !session_exists_in(space_sessions, &sid) {
                    continue;
                }
                let mut map = session_messages.write();
                let mut ui = UiState {
                    messages: map.get(&sid).cloned().unwrap_or_default(),
                };
                ui.apply(&ev);
                map.insert(sid.clone(), ui.messages);
            }

            // TurnEnd：占位文案 + 状态收尾。会话已删除则跳过消息写回
            // 与会话状态更新（写回去等于复活已删会话），但 statusline
            // 结算与 is_streaming 复位必须照常执行。
            let deleted = !session_exists_in(space_sessions, &sid);
            if !deleted {
                let mut map = session_messages.write();
                let mut ui = UiState {
                    messages: map.get(&sid).cloned().unwrap_or_default(),
                };
                ui.apply(&AgentEvent::TurnEnd {
                    stop_reason: "end_turn".into(),
                });
                map.insert(sid.clone(), ui.messages);
            }

            let mut st = statusline();
            st.tokens_out += total_out;
            st.tokens_in = (session_messages
                .read()
                .get(&sid)
                .map(|list| list.iter().map(|m| m.content.len()).sum::<usize>())
                .unwrap_or(0)
                / 4) as u64;
            st.cost_usd += total_out as f64 * 0.000002;
            st.context_pct =
                ((st.tokens_in + st.tokens_out) as f64 / st.context_max as f64 * 100.0).min(100.0);
            statusline.set(st);

            if !deleted {
                let mut map = space_sessions.read().clone();
                for list in map.values_mut() {
                    for s in list.iter_mut() {
                        if s.id == sid {
                            s.status = SessionStatus::Idle;
                            s.last_active = format_relative_time(now);
                        }
                    }
                }
                space_sessions.set(map);
            }
            is_streaming.set(false);
        });
    };

    // ── 渲染 ────────────────────────────────────────────────────────────────

    let header_title = if view() == View::Stats {
        "数据统计".to_string()
    } else {
        active_title
    };

    rsx! {
        div { class: "grid h-screen w-screen bg-base overflow-hidden relative select-none",
            style: "grid-template-columns: {grid_cols(sidebar_collapsed(), sidebar_width())};",
            onmousemove: on_root_mousemove,
            onmouseup: on_root_mouseup,
            Sidebar {
                spaces: spaces(),
                space_sessions: space_sessions.read().clone(),
                active_id: active_session_id(),
                on_select: move |id: String| {
                    active_session_id.set(id);
                    view.set(View::Chat);
                },
                on_select_space: on_select_space,
                on_create: on_create_session,
                on_delete_session: on_delete_session,
                on_delete_space: on_delete_space,
                collapsed: sidebar_collapsed(),
                on_toggle: on_toggle_sidebar,
                on_expand: on_expand_sidebar,
                width: sidebar_width(),
                on_resize_start: on_resize_start,
                on_preset_start: move |x: i32| {
                    preset_dragging.set(true);
                    let _ = x;
                },
                on_open_search: move |_| show_quick_switcher.set(true),
                on_open_stats: move |_| view.set(View::Stats),
                on_open_settings: move |_| show_settings.set(true),
            }

            // 中栏：面包屑头 + 视图
            div { class: "min-w-0 flex flex-col bg-base overflow-hidden",
                div { class: "min-h-[44px] pl-7 pr-5 pt-3 pb-2 border-b border-b1 flex items-center gap-2 shrink-0",
                    if view() == View::Stats {
                        span { class: "text-[14px] leading-5 font-medium text-label", "数据统计" }
                    } else {
                        span { class: "text-[14px] leading-5 font-medium text-label", "{active_space.name}" }
                        if !active_space.branch.is_empty() {
                            span { class: "text-caption", "/" }
                            span { class: "text-[14px] leading-5 text-label-2 truncate max-w-[360px]", "{header_title}" }
                            span { class: "font-mono text-[11px] leading-4 px-2 py-0.5 rounded-full bg-chip-brand text-brand-300 border border-b1 shrink-0",
                                "{active_space.branch}"
                            }
                        }
                    }
                    div { class: "flex-1" }
                    if is_streaming() {
                        span { class: "flex items-center gap-1.5 text-[12px] leading-5 text-label-3",
                            span { class: "spinner-ring" }
                            "运行中"
                        }
                    }
                }
                match view() {
                    View::Stats => rsx! { StatsView {} },
                    View::Chat => rsx! {
                        Chat {
                            messages: current_messages,
                            statusline: statusline(),
                            is_streaming: is_streaming(),
                            on_send: on_send,
                            on_model_change: on_model_change,
                            on_toggle_thinking: on_toggle_thinking,
                            on_toggle_tasks: move |_| show_tasks.set(!show_tasks()),
                            dock: show_tasks().then(|| rsx! {
                                TaskPanel { tasks: store::tasks(), on_close: move |_| show_tasks.set(false) }
                            }),
                        }
                    },
                }
            }

            // ⌘K 快速切换
            if show_quick_switcher() {
                Modal { width_class: "w-[560px]", top_aligned: true, on_close: move |_| show_quick_switcher.set(false),
                    div { class: "p-3 flex flex-col gap-2",
                        input {
                            class: "w-full h-11 rounded-xl bg-layer-1 border border-b2 px-4 text-[14px] leading-5 text-label outline-none transition-colors focus:border-brand placeholder:text-caption",
                            r#type: "text",
                            placeholder: "搜索会话名称或编号...",
                            value: "{search_query}",
                            oninput: move |e| search_query.set(e.value().clone()),
                            onkeydown: move |e: KeyboardEvent| {
                                if e.key() == Key::Escape {
                                    show_quick_switcher.set(false);
                                }
                            },
                            autofocus: true,
                        }
                        div { class: "max-h-[320px] overflow-y-auto flex flex-col gap-px",
                            for session in all_sessions_sorted(&spaces(), space_sessions.read().clone())
                                .iter()
                                .filter(|s| {
                                    let q = search_query().to_lowercase();
                                    q.is_empty()
                                        || s.title.to_lowercase().contains(&q)
                                        || s.id.to_lowercase().contains(&q)
                                })
                            {
                                {
                                    let id = session.id.clone();
                                    let is_active = session.id == active_session_id();
                                    let row_class = if is_active {
                                        "h-10 px-3 rounded-[10px] flex items-center justify-between cursor-pointer transition-colors bg-ihover"
                                    } else {
                                        "h-10 px-3 rounded-[10px] flex items-center justify-between cursor-pointer transition-colors hover:bg-ihover"
                                    };
                                    let dot = match session.status {
                                        SessionStatus::Active => "bg-brand",
                                        SessionStatus::Archived => "bg-danger/70",
                                        SessionStatus::Idle => "bg-dim",
                                    };
                                    rsx! {
                                        div { class: "{row_class}",
                                            onclick: move |_| {
                                                active_session_id.set(id.clone());
                                                view.set(View::Chat);
                                                show_quick_switcher.set(false);
                                            },
                                            div { class: "flex items-center gap-2.5 min-w-0",
                                                span { class: "w-2 h-2 rounded-full {dot} shrink-0" }
                                                span { class: "text-[14px] leading-5 text-label truncate", "{session.title}" }
                                            }
                                            span { class: "font-mono text-[11px] leading-4 text-caption shrink-0", "{session.id}" }
                                        }
                                    }
                                }
                            }
                        }
                        div { class: "pt-2 border-t border-b1 flex items-center justify-between text-[12px] leading-4 text-caption",
                            span { "选择会话快速切换" }
                            div { class: "flex items-center gap-1.5",
                                kbd { "ESC" }
                                span { "退出" }
                            }
                        }
                    }
                }
            }

            // 设置弹窗
            if show_settings() {
                SettingsModal {
                    config: config.clone(),
                    on_update_config: on_update_config,
                    on_close: move |_| show_settings.set(false),
                }
            }
        }
    }
}

fn grid_cols(collapsed: bool, width: usize) -> String {
    if collapsed {
        "56px minmax(0,1fr)".to_string()
    } else {
        format!("{width}px minmax(0,1fr)")
    }
}

/// 展平所有空间的会话（快速切换用），按最近活跃排序。
fn all_sessions_sorted(
    spaces: &[WorkspaceSpace],
    map: HashMap<String, Vec<Session>>,
) -> Vec<Session> {
    let order: Vec<String> = spaces.iter().map(|s| s.path.clone()).collect();
    let mut list: Vec<Session> = Vec::new();
    for path in order {
        if let Some(items) = map.get(&path) {
            list.extend(items.iter().cloned());
        }
    }
    list.sort_by_key(|s| std::cmp::Reverse(s.last_active_epoch));
    list
}
