use crate::mock::{Session, SessionStatus};
use dioxus::prelude::*;
use std::collections::HashMap;

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
    space_session_counts: HashMap<String, usize>,
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
        aside { class: "w-[260px] min-w-[260px] h-full bg-sidebar border-r border-subtle flex flex-col overflow-hidden select-none",
            // 顶层标签 tab
            div { class: "flex border-b border-subtle shrink-0",
                button {
                    class: if tab() == "spaces" { "flex-1 py-2 px-0 text-[13px] font-medium text-primary border-b-2 border-accent bg-[rgba(138,123,174,0.05)]" } else { "flex-1 py-2 px-0 text-[13px] text-muted border-b-2 border-transparent hover:text-secondary" },
                    onclick: move |_| tab.set("spaces".into()),
                    "Spaces"
                }
                button {
                    class: if tab() == "sessions" { "flex-1 py-2 px-0 text-[13px] font-medium text-primary border-b-2 border-accent bg-[rgba(138,123,174,0.05)]" } else { "flex-1 py-2 px-0 text-[13px] text-muted border-b-2 border-transparent hover:text-secondary" },
                    onclick: move |_| tab.set("sessions".into()),
                    "Sessions"
                }
            }

            if tab() == "spaces" {
                div { class: "flex-1 min-h-0 flex flex-col overflow-hidden",
                    div { class: "flex-1 overflow-y-auto flex flex-col gap-1 px-3 py-2",
                        for space in spaces {
                            {
                                let is_active = space.id == active_space_id || space.path == active_space_id;
                                let space_id = space.id.clone();
                                let session_count = space_session_counts.get(&space.path).copied().unwrap_or(0);
                                rsx! {
                                    div {
                                        key: "{space.id}",
                                        class: if is_active { "px-2.5 py-1.5 rounded-md bg-surface border border-subtle border-l-2 border-l-accent cursor-pointer transition-colors" } else { "px-2.5 py-1.5 rounded-md border border-transparent hover:bg-hover hover:border-subtle cursor-pointer transition-colors" },
                                        onclick: move |_| on_select_space.call(space_id.clone()),
                                        div { class: "flex items-center justify-between gap-2",
                                            span { class: "text-xs font-medium text-primary truncate min-w-0", "{space.name}" }
                                            div { class: "flex items-center gap-1 shrink-0",
                                                if is_active { span { class: "font-mono text-[9px] px-1 rounded-sm bg-[rgba(138,123,174,0.12)] text-accent", "active" } }
                                                span { class: "font-mono text-[9px] px-1 rounded-sm bg-[rgba(255,255,255,0.06)] text-muted", "{session_count}" }
                                            }
                                        }
                                        div { class: "font-mono text-[10px] text-secondary", "{space.branch}" }
                                        {
                                            let home = std::env::var("HOME").unwrap_or_default();
                                            let short = space.path.replacen(&home, "~", 1);
                                            rsx! { div { class: "font-mono text-[10px] text-muted truncate", "{short}" } }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                div { class: "flex-1 min-h-0 flex flex-col overflow-hidden",
                    div { class: "flex-1 overflow-y-auto flex flex-col gap-0.5 px-2 py-2",
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
                                div { class: "flex items-center gap-1.5 px-2.5 py-2 border-t border-subtle bg-[rgba(0,0,0,0.22)] shrink-0",
                                    if rename_open() {
                                        input {
                                            class: "flex-1 min-w-0 bg-base border border-accent rounded-md text-primary px-2 py-1 text-xs outline-none",
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
                                                        if !t.is_empty() { on_rename.call((id.clone(), t)); }
                                                        rename_open.set(false);
                                                    } else if e.key() == Key::Escape {
                                                        rename_open.set(false);
                                                    }
                                                }
                                            },
                                        }
                                        button {
                                            class: "p-1.5 rounded-md text-muted hover:text-primary hover:bg-hover border border-subtle",
                                            title: "确认重命名",
                                            onclick: {
                                                let id = id_for_rename.clone();
                                                let on_rename = on_rename;
                                                move |_| {
                                                    let t = rename_val().trim().to_string();
                                                    if !t.is_empty() { on_rename.call((id.clone(), t)); }
                                                    rename_open.set(false);
                                                }
                                            },
                                            "✓"
                                        }
                                    } else {
                                        button {
                                            class: "bg-transparent border border-subtle rounded-[5px] text-muted px-2 py-1 cursor-pointer transition-colors hover:text-primary hover:border-hover hover:bg-hover",
                                            title: "重命名会话",
                                            onclick: {
                                                let title = active.title.clone();
                                                move |_| {
                                                    rename_val.set(title.clone());
                                                    rename_open.set(true);
                                                }
                                            },
                                            svg { xmlns: "http://www.w3.org/2000/svg", width: "14", height: "14", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8", stroke_linecap: "round", stroke_linejoin: "round",
                                                path { d: "M12 20h9" }
                                                path { d: "M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4 12.5-12.5z" }
                                            }
                                        }
                                    }
                                    button {
                                        class: "bg-transparent border border-subtle rounded-[5px] text-muted px-2 py-1 cursor-pointer transition-colors hover:text-primary hover:border-hover hover:bg-hover",
                                        title: if active_is_archived { "取消归档" } else { "归档会话" },
                                        onclick: move |_| on_archive.call(id_for_archive.clone()),
                                        svg { xmlns: "http://www.w3.org/2000/svg", width: "14", height: "14", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8", stroke_linecap: "round", stroke_linejoin: "round",
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
                                        class: "bg-transparent border border-subtle rounded-[5px] text-muted px-2 py-1 cursor-pointer transition-colors hover:text-danger hover:border-[rgba(248,113,113,0.35)] hover:bg-[rgba(248,113,113,0.1)]",
                                        title: "删除会话",
                                        onclick: move |_| on_delete.call(id_for_delete.clone()),
                                        svg { xmlns: "http://www.w3.org/2000/svg", width: "14", height: "14", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8", stroke_linecap: "round", stroke_linejoin: "round",
                                            path { d: "M3 6h18" }
                                            path { d: "M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" }
                                            path { d: "M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6" }
                                            path { d: "M10 11v6" }
                                            path { d: "M14 11v6" }
                                        }
                                    }
                                    span { class: "ml-auto text-[10px] text-muted font-mono truncate max-w-[100px]", "{active.title}" }
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
        "inline-block w-2 h-2 rounded-full bg-accent"
    } else {
        "inline-block w-2 h-2 rounded-full bg-muted"
    };
    let id_for_select = session.id.clone();

    let row_class = if is_archived {
        if active {
            "flex items-start gap-2.5 px-2.5 py-1.5 mb-0.5 rounded-md cursor-pointer transition-colors bg-surface-elevated border border-accent opacity-60"
        } else {
            "flex items-start gap-2.5 px-2.5 py-1.5 mb-0.5 rounded-md cursor-pointer transition-colors border border-transparent opacity-60 hover:bg-hover"
        }
    } else if active {
        "flex items-start gap-2.5 px-2.5 py-1.5 mb-0.5 rounded-md cursor-pointer transition-colors bg-surface-elevated border border-accent"
    } else {
        "flex items-start gap-2.5 px-2.5 py-1.5 mb-0.5 rounded-md cursor-pointer transition-colors border border-transparent hover:bg-hover"
    };

    rsx! {
        div {
            class: "{row_class}",
            onclick: move |_| on_select.call(id_for_select.clone()),
            span { class: "{dot_class} shrink-0 mt-[5px]" }
            div { class: "flex-1 min-w-0 flex flex-col gap-0.5",
                div { class: "flex items-center justify-between gap-2",
                    span { class: "text-xs font-medium text-primary truncate flex-1", "{session.title}" }
                    span { class: "text-[10px] text-muted font-mono shrink-0", "{session.last_active}" }
                }
            }
        }
    }
}
