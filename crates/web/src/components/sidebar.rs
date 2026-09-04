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
            div { class: "sidebar-header",
                h2 { "会话" }
                button {
                    class: "btn-new-session",
                    onclick: move |_| on_create.call(()),
                    "+ 新会话"
                }
            }
            div { class: "sidebar-section",
                div { class: "sidebar-label", "进行中" }
                for session in active_sessions {
                    SessionRow {
                        key: "{session.id}",
                        session: session.clone(),
                        active: session.id == active_id,
                        on_select: on_select,
                    }
                }
            }
            if !archived.is_empty() {
                div { class: "sidebar-section",
                    div { class: "sidebar-label", "已归档" }
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
    rsx! {
        div {
            class: "{class}",
            onclick: move |_| on_select.call(id.clone()),
            span { class: "{dot_class}" }
            div { class: "session-info",
                div { class: "session-title", "{session.title}" }
                div { class: "session-meta",
                    span { "{session.model}" }
                    span { class: "session-time", "{session.last_active}" }
                }
            }
        }
    }
}
