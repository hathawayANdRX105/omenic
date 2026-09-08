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
    let mut tab = use_signal(|| "sessions".to_string());
    let mut rename_open = use_signal(|| false);
    let mut rename_val = use_signal(String::new);

    rsx! {
        aside { class: "sidebar",
            // 顶层 icon tab(可悬浮文字提示,跟随 Cursor/Claude 桌面习惯)
            div { class: "sidebar-tabs",
                button {
                    class: if tab() == "spaces" { "sidebar-tab active" } else { "sidebar-tab" },
                    title: "工作空间",
                    onclick: move |_| tab.set("spaces".into()),
                    "◇"
                }
                button {
                    class: if tab() == "sessions" { "sidebar-tab active" } else { "sidebar-tab" },
                    title: "会话",
                    onclick: move |_| tab.set("sessions".into()),
                    "▦"
                }
            }

            if tab() == "spaces" {
                div { class: "sidebar-section sidebar-fill",
                    div { class: "sidebar-section-header",
                        span { class: "sidebar-title", "Spaces" }
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
                div { class: "sidebar-section sidebar-fill",
                    div { class: "sidebar-section-header",
                        span { class: "sidebar-title", "Sessions" }
                        button {
                            class: "btn-subtle-accent",
                            onclick: move |_| on_create.call(()),
                            "+ 新建"
                        }
                    }
                    div { class: "agents-list",
                        for session in sessions.iter() {
                            SessionRow {
                                key: "{session.id}",
                                session: session.clone(),
                                active: session.id == active_id,
                                on_select: on_select,
                            }
                        }
                    }

                    {
                        let active = sessions.iter().find(|s| s.id == active_id).cloned();
                        if let Some(active) = active {
                            let id_for_rename = active.id.clone();
                            let id_for_archive = active.id.clone();
                            let id_for_delete = active.id.clone();
                            let active_is_archived = active.status == SessionStatus::Archived;
                            rsx! {
                                div { class: "sidebar-action-bar",
                                    if rename_open() {
                                        input {
                                            class: "sidebar-rename-input",
                                            value: "{rename_val}",
                                            placeholder: "新名称",
                                            oninput: move |e| rename_val.set(e.value()),
                                            autofocus: true,
                                            onkeydown: {
                                                let id = id_for_rename.clone();
                                                let on_rename = on_rename;
                                                move |e: KeyboardEvent| {
                                                    if e.key() == Key::Enter {
                                                        let t = rename_val().trim().to_string();
                                                        if !t.is_empty() {
                                                            on_rename.call((id.clone(), t));
                                                        }
                                                        rename_open.set(false);
                                                    } else if e.key() == Key::Escape {
                                                        rename_open.set(false);
                                                    }
                                                }
                                            },
                                        }
                                        button {
                                            class: "sidebar-action-icon",
                                            title: "确认重命名",
                                            onclick: {
                                                let id = id_for_rename.clone();
                                                let on_rename = on_rename;
                                                move |_| {
                                                    let t = rename_val().trim().to_string();
                                                    if !t.is_empty() {
                                                        on_rename.call((id.clone(), t));
                                                    }
                                                    rename_open.set(false);
                                                }
                                            },
                                            "✓"
                                        }
                                    } else {
                                        button {
                                            class: "sidebar-action-icon",
                                            title: "重命名会话",
                                            onclick: {
                                                let title = active.title.clone();
                                                move |_| {
                                                    rename_val.set(title.clone());
                                                    rename_open.set(true);
                                                }
                                            },
                                            svg {
                                                xmlns: "http://www.w3.org/2000/svg",
                                                width: "14",
                                                height: "14",
                                                view_box: "0 0 24 24",
                                                fill: "none",
                                                stroke: "currentColor",
                                                stroke_width: "1.8",
                                                stroke_linecap: "round",
                                                stroke_linejoin: "round",
                                                path { d: "M12 20h9" }
                                                path { d: "M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4 12.5-12.5z" }
                                            }
                                        }
                                    }
                                    button {
                                        class: "sidebar-action-icon",
                                        title: if active_is_archived { "取消归档" } else { "归档会话" },
                                        onclick: move |_| on_archive.call(id_for_archive.clone()),
                                        svg {
                                            xmlns: "http://www.w3.org/2000/svg",
                                            width: "14",
                                            height: "14",
                                            view_box: "0 0 24 24",
                                            fill: "none",
                                            stroke: "currentColor",
                                            stroke_width: "1.8",
                                            stroke_linecap: "round",
                                            stroke_linejoin: "round",
                                            rect { x: "3", y: "3", width: "18", height: "5", rx: "1" }
                                            path { d: "M5 8v10a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8" }
                                            if active_is_archived {
                                                path { d: "M9 14l3-3 3 3" }
                                                path { d: "M12 11v7" }
                                            } else {
                                                path { d: "M10 12h4" }
                                            }
                                        }
                                    }
                                    button {
                                        class: "sidebar-action-icon danger",
                                        title: "删除会话",
                                        onclick: move |_| on_delete.call(id_for_delete.clone()),
                                        svg {
                                            xmlns: "http://www.w3.org/2000/svg",
                                            width: "14",
                                            height: "14",
                                            view_box: "0 0 24 24",
                                            fill: "none",
                                            stroke: "currentColor",
                                            stroke_width: "1.8",
                                            stroke_linecap: "round",
                                            stroke_linejoin: "round",
                                            path { d: "M3 6h18" }
                                            path { d: "M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" }
                                            path { d: "M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6" }
                                            path { d: "M10 11v6" }
                                            path { d: "M14 11v6" }
                                        }
                                    }
                                    span { class: "sidebar-action-hint", "{active.title}" }
                                }
                            }
                        } else {
                            rsx! {}
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn SessionRow(session: Session, active: bool, on_select: EventHandler<String>) -> Element {
    let is_run = session.status == SessionStatus::Active;
    let is_archived = session.status == SessionStatus::Archived;
    let dot_class = if is_run {
        "status-dot run"
    } else {
        "status-dot idle"
    };
    let status_label = if is_run { "RUN" } else { "IDLE" };
    let id_for_select = session.id.clone();

    rsx! {
        div {
            class: if is_archived {
                if active { "session-row active archived" } else { "session-row archived" }
            } else if active {
                "session-row active"
            } else {
                "session-row"
            },
            onclick: move |_| on_select.call(id_for_select.clone()),
            div { class: "session-dot-col",
                span { class: "{dot_class}" }
            }
            div { class: "session-content",
                div { class: "session-title-row",
                    span { class: "session-title", "{session.title}" }
                    span { class: "session-tag-chip", "{status_label}" }
                }
                div { class: "session-meta-row",
                    span { class: "session-time", "{session.last_active}" }
                }
            }
        }
    }
}
