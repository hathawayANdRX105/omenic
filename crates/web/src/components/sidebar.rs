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
        .filter(|s| matches!(s.status, SessionStatus::Active | SessionStatus::Idle))
        .collect();
    let archived: Vec<_> = sessions
        .iter()
        .filter(|s| matches!(s.status, SessionStatus::Archived))
        .collect();

    rsx! {
        aside { class: "sidebar",
            // Header with "+ 新建任务" button
            div { class: "sidebar-header",
                button {
                    class: "btn-new-session",
                    onclick: move |_| on_create.call(()),
                    span { style: "font-weight: 600; font-size: 14px; line-height: 1;", "+" }
                    span { "新建任务" }
                }
            }

            // Sessions list section
            div { class: "sidebar-section",
                div { class: "sidebar-label", "任务 ({sessions.len()})" }
                for session in active_sessions {
                    SessionRow {
                        key: "{session.id}",
                        session: session.clone(),
                        active: session.id == active_id,
                        on_select: on_select,
                    }
                }

                if !archived.is_empty() {
                    div { class: "sidebar-label", style: "margin-top: 14px;", "已归档" }
                    for session in archived {
                        SessionRow {
                            key: "{session.id}",
                            session: session.clone(),
                            active: session.id == active_id,
                            on_select: on_select,
                        }
                    }
                }
            }

            // Bottom status bar
            div { class: "sidebar-user-footer",
                div { class: "user-profile",
                    span { class: "user-status-dot" }
                    span { class: "user-name", "omenic agent" }
                }
                span { style: "font-size: 11px; color: var(--text-muted); font-family: monospace;", "ready" }
            }
        }
    }
}

#[component]
fn SessionRow(session: Session, active: bool, on_select: EventHandler<String>) -> Element {
    let class = if active {
        "session-row active"
    } else {
        "session-row"
    };
    let dot_class = match session.status {
        SessionStatus::Active => "session-dot active",
        SessionStatus::Idle => "session-dot idle",
        SessionStatus::Archived => "session-dot archived",
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
            span { class: "{dot_class}" }
            div { class: "session-info",
                div { class: "session-title", "{session.title}" }
                div { class: "session-meta",
                    span { class: "session-tag", "{tag}" }
                    span { "{session.last_active}" }
                }
            }
        }
    }
}
