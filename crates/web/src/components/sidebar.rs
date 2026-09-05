use crate::mock::{Session, SessionStatus};
use dioxus::prelude::*;

#[component]
pub fn Sidebar(
    sessions: Vec<Session>,
    active_id: String,
    on_select: EventHandler<String>,
    on_create: EventHandler<()>,
) -> Element {
    let active_sessions: Vec<_> = sessions
        .iter()
        .filter(|s| matches!(s.status, SessionStatus::Active))
        .collect();
    let idle_sessions: Vec<_> = sessions
        .iter()
        .filter(|s| matches!(s.status, SessionStatus::Idle))
        .collect();
    let running_count = active_sessions.len();
    let total_count = sessions.len();

    rsx! {
        aside { class: "sidebar",
            // Spaces section (top half)
            div { class: "sidebar-spaces-section",
                div { class: "sidebar-section-header",
                    span { class: "sidebar-title", "SPACES" }
                    button {
                        class: "btn-open-switch",
                        onclick: move |_| {
                            // Placeholder for open/switch action
                        },
                        "[+ 打开]"
                    }
                }
                div { class: "spaces-list",
                    // Current project card
                    div { class: "space-card active",
                        span { class: "space-name", "omenic (main)" }
                        span { class: "space-path", "~/projects/omenic" }
                        span { class: "space-branch", "git: feat/web-agent-harness" }
                    }
                    // Web agent harness card
                    div { class: "space-card",
                        span { class: "space-name", ".wt/web-agent-harness" }
                        span { class: "space-branch", "git: feat/web-agent-harness" }
                    }
                }
            }

            // Divider
            div { class: "sidebar-divider" }

            // Agents section (bottom half)
            div { class: "sidebar-agents-section",
                div { class: "sidebar-section-header",
                    span { class: "sidebar-title", "AGENTS" }
                    button {
                        class: "btn-new-agent",
                        onclick: move |_| on_create.call(()),
                        "[+ 新建会话]"
                    }
                }
                // Filter bar
                div { class: "agents-filter-bar",
                    button { class: "filter-btn active", "正在运行 ({running_count})" }
                    button { class: "filter-btn", "全部 ({total_count})" }
                }
                // Sessions list (scrollable)
                div { class: "agents-list",
                    for session in active_sessions {
                        SessionRow {
                            key: "{session.id}",
                            session: session.clone(),
                            active: session.id == active_id,
                            on_select: on_select,
                            status: "RUN".to_string(),
                        }
                    }
                    for session in idle_sessions {
                        SessionRow {
                            key: "{session.id}",
                            session: session.clone(),
                            active: session.id == active_id,
                            on_select: on_select,
                            status: "IDLE".to_string(),
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
    status: String,
) -> Element {
    let class = if active {
        "session-row active"
    } else {
        "session-row"
    };
    let dot = match status.as_str() {
        "RUN" => "●",
        _ => "○",
    };
    let dot_class = match status.as_str() {
        "RUN" => "session-dot run",
        _ => "session-dot idle",
    };
    let id = session.id.clone();
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
            onclick: move |_| on_select.call(id.clone()),
            span { class: "{dot_class}" }{dot}
            div { class: "session-info",
                div { class: "session-title", "{session.title}" }
                div { class: "session-meta",
                    span { class: "session-tag", "{tag}" }
                    span { class: "session-status", "{status}" }
                    span { "{session.last_active}" }
                }
            }
        }
    }
}
