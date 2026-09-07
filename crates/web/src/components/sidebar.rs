use crate::mock::{Session, SessionStatus};
use dioxus::prelude::*;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceSpace {
    pub id: String,
    pub name: String,
    pub path: String,
    pub branch: String,
    pub is_active: bool,
}

#[component]
pub fn Sidebar(
    spaces: Vec<WorkspaceSpace>,
    active_space_id: String,
    on_select_space: EventHandler<String>,
    on_trigger_picker: EventHandler<()>,
    sessions: Vec<Session>,
    active_id: String,
    on_select: EventHandler<String>,
    on_create: EventHandler<()>,
    on_delete: EventHandler<String>,
    on_archive: EventHandler<String>,
    on_rename: EventHandler<(String, String)>,
) -> Element {
    // 顶部图标 tab：🗂 工作空间 / 💬 会话
    let mut tab = use_signal(|| "sessions".to_string());

    rsx! {
        aside { class: "sidebar",
            // Top icon tab bar (合并 SPACES 与 AGENTS 入口)
            div { class: "sidebar-tabs",
                button {
                    class: if tab() == "spaces" { "sidebar-tab active" } else { "sidebar-tab" },
                    title: "工作空间",
                    onclick: move |_| tab.set("spaces".into()),
                    "🗂"
                }
                button {
                    class: if tab() == "sessions" { "sidebar-tab active" } else { "sidebar-tab" },
                    title: "会话",
                    onclick: move |_| tab.set("sessions".into()),
                    "💬"
                }
            }

            if tab() == "spaces" {
                div { class: "sidebar-section sidebar-spaces-section sidebar-fill",
                    div { class: "sidebar-section-header",
                        span { class: "sidebar-title", "SPACES" }
                        button {
                            class: "btn-subtle",
                            onclick: move |_| on_trigger_picker.call(()),
                            "打开"
                        }
                    }
                    div { class: "spaces-list",
                        for space in spaces {
                            {
                                let is_active = space.id == active_space_id || space.path == active_space_id;
                                let space_id = space.id.clone();
                                rsx! {
                                    div {
                                        key: "{space.id}",
                                        class: if is_active { "space-card active" } else { "space-card" },
                                        onclick: move |_| on_select_space.call(space_id.clone()),
                                        div { class: "space-card-top",
                                            span { class: "space-name", "{space.name}" }
                                            if is_active { span { class: "space-badge", "active" } }
                                        }
                                        div { class: "space-branch", "{space.branch}" }
                                        {
                                            let home = std::env::var("HOME").unwrap_or_default();
                                            let short = space.path.replacen(&home, "~", 1);
                                            rsx! { span { class: "space-path", "{short}" } }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                div { class: "sidebar-section sidebar-agents-section sidebar-fill",
                    div { class: "sidebar-section-header",
                        span { class: "sidebar-title", "会话" }
                        button {
                            class: "btn-subtle-accent",
                            onclick: move |_| on_create.call(()),
                            "+ 新建"
                        }
                    }
                    div { class: "agents-list",
                        for session in sessions {
                            SessionRow {
                                key: "{session.id}",
                                session: session.clone(),
                                active: session.id == active_id,
                                on_select: on_select,
                                on_delete: on_delete,
                                on_archive: on_archive,
                                on_rename: on_rename,
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn SessionRow(
    session: Session,
    active: bool,
    on_select: EventHandler<String>,
    on_delete: EventHandler<String>,
    on_archive: EventHandler<String>,
    on_rename: EventHandler<(String, String)>,
) -> Element {
    let class = if active {
        "session-row active"
    } else {
        "session-row"
    };
    let is_run = session.status == SessionStatus::Active;
    let is_archived = session.status == SessionStatus::Archived;
    let dot_class = if is_run {
        "status-dot run"
    } else {
        "status-dot idle"
    };
    let status_label = if is_run { "RUN" } else { "IDLE" };

    let id_for_select = session.id.clone();
    let id_delete = session.id.clone();
    let id_archive = session.id.clone();
    let id_rename = session.id.clone();

    let mut edit_mode = use_signal(|| false);
    let mut edit_text = use_signal(|| session.title.clone());
    let title_for_submit = session.title.clone();

    let tag = if session.title.contains("orbit") {
        "orbit"
    } else if session.title.contains("MCP") {
        "mcp"
    } else if session.title.contains("memory") {
        "memory"
    } else if session.title.contains("TUI") {
        "tui"
    } else {
        "task"
    };

    rsx! {
        div {
            class: if is_archived {
                if active { "session-row active archived" } else { "session-row archived" }
            } else if active {
                "session-row active"
            } else {
                "session-row"
            },
            div {
                class: "session-main-click",
                onclick: move |_| {
                    if !edit_mode() { on_select.call(id_for_select.clone()); }
                },
                div { class: "session-dot-col",
                    span { class: "{dot_class}" }
                }
                div { class: "session-content",
                    if edit_mode() {
                        input {
                            class: "session-rename-input",
                            r#type: "text",
                            value: "{edit_text}",
                            oninput: move |e| edit_text.set(e.value()),
                            onkeydown: {
                                let id_for_key = id_rename.clone();
                                let on_rename_key = on_rename;
                                let title0 = session.title.clone();
                                move |e: KeyboardEvent| {
                                    if e.key() == Key::Enter {
                                        let t = edit_text().trim().to_string();
                                        if !t.is_empty() && t != title0 {
                                            on_rename_key.call((id_for_key.clone(), t));
                                        }
                                        edit_mode.set(false);
                                    }
                                }
                            },
                            onblur: {
                                let id_for_blur = id_rename.clone();
                                let on_rename_blur = on_rename;
                                let title0 = session.title.clone();
                                move |_| {
                                    let t = edit_text().trim().to_string();
                                    if !t.is_empty() && t != title0 {
                                        on_rename_blur.call((id_for_blur.clone(), t));
                                    }
                                    edit_mode.set(false);
                                }
                            },
                            autofocus: true,
                        }
                    } else {
                        div { class: "session-title-row",
                            span { class: "session-title", "{session.title}" }
                            span { class: "session-tag-chip", "{status_label}" }
                        }
                        div { class: "session-meta-row",
                            span { "{tag}" }
                            span { class: "session-time", "{session.last_active}" }
                        }
                    }
                }
            }
            div { class: "session-actions",
                button {
                    class: "session-action-btn",
                    title: "重命名",
                    onclick: move |e: MouseEvent| {
                        e.stop_propagation();
                        edit_text.set(session.title.clone());
                        edit_mode.set(true);
                    },
                    "✎"
                }
                button {
                    class: "session-action-btn",
                    title: if is_archived { "取消归档" } else { "归档" },
                    onclick: move |e: MouseEvent| {
                        e.stop_propagation();
                        on_archive.call(id_archive.clone());
                    },
                    if is_archived { "📤" } else { "📥" }
                }
                button {
                    class: "session-action-btn danger",
                    title: "删除会话",
                    onclick: move |e: MouseEvent| {
                        e.stop_propagation();
                        on_delete.call(id_delete.clone());
                    },
                    "🗑"
                }
            }
        }
    }
}
