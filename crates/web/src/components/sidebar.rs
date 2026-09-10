use crate::components::ui::{Button, ButtonVariant, IconButton};
use crate::mock::{Session, SessionStatus};
use dioxus::prelude::*;
use std::collections::{HashMap, HashSet};

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
    space_sessions: HashMap<String, Vec<Session>>,
    active_id: String,
    on_select: EventHandler<String>,
    on_create: EventHandler<()>,
    on_delete: EventHandler<String>,
    on_archive: EventHandler<String>,
    on_rename: EventHandler<(String, String)>,
) -> Element {
    // Accordion state: which space sections are expanded. Default to the active
    // space so the user lands on their current context.
    let mut expanded = use_signal(|| {
        let mut s = HashSet::new();
        s.insert(active_space_id.clone());
        s
    });
    let mut rename_open = use_signal(|| false);
    let mut rename_val = use_signal(String::new);

    let active = space_sessions
        .values()
        .flatten()
        .find(|s| s.id == active_id)
        .cloned();

    rsx! {
        aside { class: "w-[260px] min-w-[260px] h-full bg-sidebar border-r border-subtle flex flex-col overflow-hidden select-none",
            // 头部：目录标题 + 新建会话按钮（复用统一 Button）
            div { class: "flex items-center justify-between gap-2 px-3 h-[44px] border-b border-subtle shrink-0",
                span { class: "text-[13px] font-semibold text-primary", "目录" }
                Button {
                    variant: ButtonVariant::Primary,
                    onclick: move |_| on_create.call(()),
                    class: "text-[12px] px-2 py-1",
                    "新建会话"
                }
            }

            // 折叠式目录：每个 space 是一个可展开的 tab，展开后放对应会话
            div { class: "flex-1 min-h-0 overflow-y-auto flex flex-col",
                for space in spaces {
                    {
                        let space_id = space.id.clone();
                        let space_path = space.path.clone();
                        let is_open = expanded().contains(&space_id);
                        let is_active = space.id == active_space_id || space.path == active_space_id;
                        let count = space_sessions.get(&space_path).map(|s| s.len()).unwrap_or(0);
                                    let sessions_for_space = space_sessions
                                        .get(&space_path)
                                        .cloned()
                                        .unwrap_or_default();
                                    let is_empty = sessions_for_space.is_empty();
                                    let space_id_toggle = space_id.clone();
                        let space_path_select = space_path.clone();
                        rsx! {
                            div { class: "border-b border-subtle/60",
                                // 目录(tTab) 头部：点开/收起 + 选中工作区
                                button {
                                    class: if is_active {
                                        "w-full flex items-center gap-2 px-3 py-2 text-left cursor-pointer bg-[rgba(138,123,174,0.06)] border-l-2 border-l-accent"
                                    } else {
                                        "w-full flex items-center gap-2 px-3 py-2 text-left cursor-pointer hover:bg-hover border-l-2 border-l-transparent"
                                    },
                                    onclick: move |_| {
                                        let mut set = expanded.write();
                                        if set.contains(&space_id_toggle) {
                                            set.remove(&space_id_toggle);
                                        } else {
                                            set.insert(space_id_toggle.clone());
                                        }
                                        drop(set);
                                        on_select_space.call(space_path_select.clone());
                                    },
                                    span { class: "text-[13px] font-medium text-primary truncate min-w-0 flex-1", "{space.name}" }
                                    span { class: "font-mono text-[9px] px-1 rounded-sm bg-[rgba(255,255,255,0.06)] text-muted shrink-0", "{count}" }
                                }

                                if is_open {
                                    div { class: "flex flex-col gap-0.5 px-2 pb-2",
                                        for session in sessions_for_space {
                                            SessionRow {
                                                key: "{session.id}",
                                                session: session.clone(),
                                                active: session.id == active_id,
                                                on_select: on_select,
                                            }
                                        }
                                        if is_empty {
                                            div { class: "px-3 py-2 text-[11px] text-muted", "暂无会话" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // 活动会话操作条（重命名 / 归档 / 删除），复用统一 IconButton
            if let Some(active) = active {
                {
                    let id_for_rename = active.id.clone();
                    let id_for_archive = active.id.clone();
                    let id_for_delete = active.id.clone();
                    let active_is_archived = active.status == SessionStatus::Archived;
                    let title = active.title.clone();
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
                                IconButton {
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
                                    svg { xmlns: "http://www.w3.org/2000/svg", width: "14", height: "14", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8", stroke_linecap: "round", stroke_linejoin: "round",
                                        path { d: "M20 6L9 17l-5-5" }
                                    }
                                }
                            } else {
                                IconButton {
                                    title: "重命名会话",
                                    onclick: move |_| {
                                        rename_val.set(title.clone());
                                        rename_open.set(true);
                                    },
                                    svg { xmlns: "http://www.w3.org/2000/svg", width: "14", height: "14", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.8", stroke_linecap: "round", stroke_linejoin: "round",
                                        path { d: "M12 20h9" }
                                        path { d: "M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4 12.5-12.5z" }
                                    }
                                }
                            }
                            IconButton {
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
                            IconButton {
                                variant: ButtonVariant::Danger,
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
