use crate::mock::{Session, SessionStatus};
use dioxus::prelude::*;

#[component]
pub fn Sidebar(
    sessions: Vec<Session>,
    active_id: String,
    on_select: EventHandler<String>,
    on_create: EventHandler<()>,
) -> Element {
    let mut show_only_running = use_signal(|| false);

    let active_sessions: Vec<_> = sessions
        .iter()
        .filter(|s| matches!(s.status, SessionStatus::Active))
        .cloned()
        .collect();
    let running_count = active_sessions.len();
    let total_count = sessions.len();

    let display_sessions = if show_only_running() {
        active_sessions
    } else {
        sessions.clone()
    };

    rsx! {
        aside { class: "sidebar",
            // Spaces section (top half, Image #1 style)
            div { class: "sidebar-spaces-section",
                div { class: "sidebar-section-header",
                    span { class: "sidebar-title", "SPACES" }
                    button {
                        class: "btn-subtle",
                        onclick: move |_| {},
                        "打开"
                    }
                }
                div { class: "spaces-list",
                    div { class: "space-card active",
                        div { class: "space-card-top",
                            span { class: "space-name", "web-agent-harness" }
                            span { class: "space-badge", "active" }
                        }
                        span { class: "space-branch", "feat/web-agent-harness" }
                        span { class: "space-path", ".wt/web-agent-harness" }
                    }
                    div { class: "space-card",
                        div { class: "space-card-top",
                            span { class: "space-name", "omenic" }
                        }
                        span { class: "space-branch", "main" }
                        span { class: "space-path", "~/projects/omenic" }
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

                // Filter bar: 运行中 vs 全部
                div { class: "agents-filter-bar",
                    button {
                        class: if show_only_running() { "filter-tab active" } else { "filter-tab" },
                        onclick: move |_| show_only_running.set(true),
                        "运行中 {running_count}"
                    }
                    button {
                        class: if !show_only_running() { "filter-tab active" } else { "filter-tab" },
                        onclick: move |_| show_only_running.set(false),
                        "全部 {total_count}"
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
                        }
                    }
                }
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
    }
}
