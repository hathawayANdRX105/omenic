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
) -> Element {
    let display_sessions = sessions.clone();

    rsx! {
        aside { class: "sidebar",
            // Spaces section (top half, Image #1 style)
            div { class: "sidebar-spaces-section",
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
                            // 严格单选：只允许当前选中的这唯独一个空间激活
                            let is_active = space.id == active_space_id || space.path == active_space_id;
                            let space_id = space.id.clone();
                            rsx! {
                                div {
                                    key: "{space.id}",
                                    class: if is_active { "space-card active" } else { "space-card" },
                                    onclick: move |_| on_select_space.call(space_id.clone()),
                                    div { class: "space-card-top",
                                        span { class: "space-name", "{space.name}" }
                                        if is_active {
                                            span { class: "space-badge", "active" }
                                        }
                                    }
                                    span { class: "space-branch", "{space.branch}" }
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

            // Divider
            div { class: "sidebar-divider" }

            // Agents section (bottom half, Herdr Image #2 style)
            div { class: "sidebar-agents-section",
                div { class: "sidebar-section-header",
                    span { class: "sidebar-title", "AGENTS" }
                    button {
                        class: "btn-subtle-accent",
                        onclick: move |_| on_create.call(()),
                        "+ 新建"
                    }
                }

                // Sessions list
                div { class: "agents-list",
                    for session in display_sessions {
                        SessionRow {
                            key: "{session.id}",
                            session: session.clone(),
                            active: session.id == active_id,
                            on_select: on_select,
                            on_delete: on_delete,
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
) -> Element {
    let class = if active {
        "session-row active"
    } else {
        "session-row"
    };

    let is_run = session.status == SessionStatus::Active;
    let dot_class = if is_run {
        "status-dot run"
    } else {
        "status-dot idle"
    };
    let tag_class = if is_run {
        "session-tag-chip run"
    } else {
        "session-tag-chip"
    };
    let status_label = if is_run { "RUN" } else { "IDLE" };

    let id_for_select = session.id.clone();
    let id_for_delete = session.id.clone();
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
            class: "{class}",
            div {
                class: "session-main-click",
                onclick: move |_| on_select.call(id_for_select.clone()),
                div { class: "session-dot-col",
                    span { class: "{dot_class}" }
                }
                div { class: "session-content",
                    div { class: "session-title-row",
                        span { class: "session-title", "{session.title}" }
                        span { class: "{tag_class}", "{status_label}" }
                    }
                    div { class: "session-meta-row",
                        span { "{tag}" }
                        span { class: "session-time", "{session.last_active}" }
                    }
                }
            }
            button {
                class: "btn-delete-session",
                title: "删除会话",
                onclick: move |e: MouseEvent| {
                    e.stop_propagation();
                    on_delete.call(id_for_delete.clone());
                },
                "×"
            }
        }
    }
}
