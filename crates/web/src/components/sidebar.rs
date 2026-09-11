use crate::components::ui::{Badge, BadgeVariant};
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
    on_select_space: EventHandler<String>,
    on_trigger_picker: EventHandler<()>,
    space_sessions: HashMap<String, Vec<Session>>,
    active_id: String,
    on_select: EventHandler<String>,
    on_create: EventHandler<String>,
    on_delete_session: EventHandler<String>,
    on_delete_space: EventHandler<String>,
    collapsed: bool,
    on_toggle: EventHandler<()>,
    width: usize,
    on_resize_start: EventHandler<i32>,
    on_preset_start: EventHandler<i32>,
    on_expand: EventHandler<()>,
) -> Element {
    // 树状三层：L1 项目（分组标题）→ L2 项目目录（真实名称）→ L3 会话。
    // 默认全部展开，使每一行（项目与会话）都可见、可点击。
    let mut expanded_spaces =
        use_signal(|| spaces.iter().map(|s| s.id.clone()).collect::<HashSet<_>>());

    let aside_style = if collapsed {
        "width:36px;min-width:36px;".to_string()
    } else {
        format!("width:{width}px;min-width:{width}px;")
    };

    rsx! {
        aside { class: "relative h-full bg-sidebar border-r border-subtle flex flex-col shrink-0 select-none",
            style: "{aside_style}",
            if collapsed {
                // 收缩态：顶部「展开」按钮 + 各项目的会话常驻显示（小圆点表示状态）
                // 点会话只切换选中，保持折叠，不弹面板
                div { class: "pt-2 flex flex-col items-center gap-1.5 overflow-y-auto flex-1 w-full",
                    button {
                        class: "w-7 h-7 flex items-center justify-center rounded-md text-muted-foreground hover:bg-hover hover:text-foreground transition-colors",
                        title: "展开侧边栏",
                        onclick: move |_| on_expand.call(()),
                        PanelIcon {}
                    }
                    div { class: "my-1 w-5 h-px bg-subtle" }
                    for space in spaces {
                        {
                            let sp_name = space.name.clone();
                            let sp_sessions = space_sessions.get(&space.path).cloned().unwrap_or_default();
                            rsx! {
                                for s in sp_sessions {
                                    {
                                        let sid = s.id.clone();
                                        let stitle = s.title.clone();
                                        let slast = s.last_active.clone();
                                        let is_selected = s.id == active_id;
                                        let (dot_color, status_label) = if s.status == SessionStatus::Active {
                                            ("bg-accent", "运行中")
                                        } else if s.status == SessionStatus::Archived {
                                            ("bg-destructive/70", "堵塞")
                                        } else {
                                            ("bg-muted", "完成")
                                        };
                                        // 图标按钮：状态圆点；选中 = 与悬停一致的背景；悬停弹自绘信息面板；点击仅切换选中（保持折叠）
                                        rsx! {
                                            div { class: "relative group",
                                                button {
                                                    class: if is_selected {
                                                        "w-7 h-7 flex items-center justify-center rounded-md cursor-pointer bg-hover transition-colors"
                                                    } else {
                                                        "w-7 h-7 flex items-center justify-center rounded-md cursor-pointer hover:bg-hover transition-colors"
                                                    },
                                                    onclick: move |_| {
                                                        on_select.call(sid.clone());
                                                    },
                                                    span { class: "w-2.5 h-2.5 rounded-full {dot_color}" }
                                                }
                                                // 悬停信息面板（纯 CSS group-hover，pointer-events-none 防闪烁）
                                                div { class: "absolute left-full top-1/2 -translate-y-1/2 ml-2 z-50 w-52 rounded-lg border border-subtle bg-surface-elevated px-3 py-2 shadow-[0_8px_26px_rgba(0,0,0,0.42)] opacity-0 pointer-events-none group-hover:opacity-100 transition-opacity duration-150",
                                                    div { class: "text-[12px] font-medium text-foreground truncate", "{stitle}" }
                                                    div { class: "mt-1 flex items-center justify-between gap-2",
                                                        span { class: "text-[10px] text-muted-foreground truncate", "{sp_name}" }
                                                        span { class: "text-[10px] text-muted font-mono shrink-0", "{slast}" }
                                                    }
                                                    div { class: "mt-1.5 flex items-center gap-1.5",
                                                        span { class: "w-1.5 h-1.5 rounded-full {dot_color}" }
                                                        span { class: "text-[10px] text-muted-foreground", "{status_label}" }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // 预设把手：设定默认展开宽度（立即展开）
                    div {
                        class: "mt-2 mx-1 h-px w-6 bg-subtle cursor-ew-resize hover:bg-accent",
                        title: "拖拽设定默认宽度",
                        onmousedown: move |e: MouseEvent| on_preset_start.call(e.client_coordinates().x as i32),
                    }
                }
            } else {
                // 完整内容
                div { class: "flex-1 min-h-0 overflow-y-auto flex flex-col",
                    // L1 项目（分组标题）+ 收起按钮
                    div { class: "flex items-center justify-between px-3 pt-3 pb-1",
                        div { class: "text-[11px] font-semibold text-muted-foreground tracking-wider select-none", "项目" }
                        button {
                            class: "p-1 rounded-md text-muted-foreground hover:bg-hover hover:text-foreground transition-colors",
                            title: "收起侧边栏",
                            onclick: move |_| on_toggle.call(()),
                            PanelIcon {}
                        }
                    }

                    for space in spaces {
                        {
                            let space_id = space.id.clone();
                            let space_path = space.path.clone();
                            let space_path_create = space_path.clone();
                            let space_path_delete = space_path.clone();
                            let is_open = expanded_spaces().contains(&space_id);
                            let count = space_sessions.get(&space_path).map(|s| s.len()).unwrap_or(0);
                            let sessions_for_space = space_sessions.get(&space_path).cloned().unwrap_or_default();
                            let is_empty = sessions_for_space.is_empty();
                            let space_id_toggle = space_id.clone();
                            let space_path_select = space_path.clone();
                            rsx! {
                                // L2 项目目录（真实名称）：可点击 + hover 图标按钮。
                                // 项目不显示选中背景，仅 hover。数字放最右，按钮在数字左侧，不遮挡。
                                div { class: "group flex items-center gap-2 pl-4 pr-2 py-2 mx-1 hover:bg-hover cursor-pointer rounded-md transition-colors",
                                    onclick: move |_| {
                                        let mut set = expanded_spaces.write();
                                        if set.contains(&space_id_toggle) {
                                            set.remove(&space_id_toggle);
                                        } else {
                                            set.insert(space_id_toggle.clone());
                                        }
                                        drop(set);
                                        on_select_space.call(space_path_select.clone());
                                    },
                                    FolderIcon { class: "w-4 h-4 shrink-0 text-muted-foreground" }
                                    span { class: "text-[13px] font-medium text-foreground truncate min-w-0 flex-1", "{space.name}" }
                                    // hover 才出现的图标按钮（在数字左侧，不遮挡数字；无文字，带 tooltip）
                                    button {
                                        class: "shrink-0 p-1 rounded-md text-muted-foreground hover:bg-accent/20 hover:text-foreground transition-colors opacity-0 group-hover:opacity-100",
                                        title: "新建会话",
                                        onclick: move |e: MouseEvent| {
                                            e.stop_propagation();
                                            on_create.call(space_path_create.clone());
                                        },
                                        svg { width: "13", height: "13", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "2", stroke_linecap: "round", stroke_linejoin: "round",
                                            path { d: "M12 5v14" }
                                            path { d: "M5 12h14" }
                                        }
                                    }
                                    button {
                                        class: "shrink-0 p-1 rounded-md text-muted-foreground hover:bg-destructive/15 hover:text-destructive transition-colors opacity-0 group-hover:opacity-100",
                                        title: "删除项目",
                                        onclick: move |e: MouseEvent| {
                                            e.stop_propagation();
                                            on_delete_space.call(space_path_delete.clone());
                                        },
                                        svg { width: "13", height: "13", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "2", stroke_linecap: "round", stroke_linejoin: "round",
                                            path { d: "M3 6h18" }
                                            path { d: "M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" }
                                            path { d: "M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6" }
                                            path { d: "M10 11v6" }
                                            path { d: "M14 11v6" }
                                        }
                                    }
                                    // 会话数小数字放最右
                                    Badge { variant: BadgeVariant::Secondary, class: "font-mono text-[9px] px-1", "{count}" }
                                }
                                // L3 会话（项目目录的子节点，缩进 + 引导线）
                                if is_open {
                                    div { class: "ml-5 border-l border-subtle/50 pl-2 pb-1",
                                        for session in sessions_for_space {
                                            SessionRow {
                                                key: "{session.id}",
                                                session: session.clone(),
                                                active: session.id == active_id,
                                                on_select: on_select,
                                                on_delete: on_delete_session,
                                            }
                                        }
                                        if is_empty {
                                            div { class: "px-2 py-1.5 text-[11px] text-muted", "暂无会话" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                // 右边缘可拖拽边界：hover 显示拖拽样式，按下开始调整宽度
                div {
                    class: "absolute top-0 right-0 h-full w-1.5 cursor-col-resize hover:bg-accent/40 transition-colors",
                    onmousedown: move |e: MouseEvent| on_resize_start.call(e.client_coordinates().x as i32),
                }
            }
        }
    }
}

/// 文件夹图标（用于 L2 项目目录行与收缩态缩略按钮）。
#[component]
fn FolderIcon(class: String) -> Element {
    rsx! {
        svg { class: "{class}", xmlns: "http://www.w3.org/2000/svg", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.6", stroke_linecap: "round", stroke_linejoin: "round",
            path { d: "M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.909l-.06-.062a1 1 0 0 0-1.35-.927l-.4.231a2 2 0 0 1-1.69.908H4a2 2 0 0 0-2 2v10a2 2 0 0 0 2 2Z" }
        }
    }
}

/// 侧边栏收起/展开图标（面板图标，避免用三角符号）。
#[component]
fn PanelIcon() -> Element {
    rsx! {
        svg { class: "w-4 h-4", xmlns: "http://www.w3.org/2000/svg", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "2", stroke_linecap: "round", stroke_linejoin: "round",
            rect { x: "3", y: "3", width: "18", height: "18", rx: "2" }
            path { d: "M9 3v18" }
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
    let is_run = session.status == SessionStatus::Active;
    let is_archived = session.status == SessionStatus::Archived;
    let id_for_select = session.id.clone();
    let id_for_delete = session.id.clone();

    // 前导圆点：运行中=accent，堵塞(归档)=暗红，完成=灰；选中时圆点下加下划线。
    let dot_color = if is_run {
        "bg-accent"
    } else if is_archived {
        "bg-destructive/70"
    } else {
        "bg-muted"
    };

    // 选中态背景与悬停一致（bg-hover）；项目不选中变色。
    let row_class = if is_archived {
        if active {
            "group flex items-center gap-2 px-2 py-1.5 mb-0.5 rounded-md cursor-pointer transition-colors bg-hover opacity-60"
        } else {
            "group flex items-center gap-2 px-2 py-1.5 mb-0.5 rounded-md cursor-pointer transition-colors hover:bg-hover opacity-60"
        }
    } else if active {
        "group flex items-center gap-2 px-2 py-1.5 mb-0.5 rounded-md cursor-pointer transition-colors bg-hover"
    } else {
        "group flex items-center gap-2 px-2 py-1.5 mb-0.5 rounded-md cursor-pointer transition-colors hover:bg-hover"
    };

    rsx! {
        div {
            class: "{row_class}",
            onclick: move |_| on_select.call(id_for_select.clone()),
            // 前导圆点（状态三色）
            span { class: "w-2 h-2 rounded-full {dot_color} shrink-0" }
            // 标题垂直居中
            span { class: "text-xs font-medium text-foreground truncate flex-1", "{session.title}" }
            // hover 才出现的删除按钮（在时间左侧，不遮挡时间；无文字，带 tooltip）
            button {
                class: "shrink-0 p-1 rounded-md text-muted-foreground opacity-0 group-hover:opacity-100 hover:bg-destructive/15 hover:text-destructive transition-all",
                title: "删除会话",
                onclick: move |e: MouseEvent| {
                    e.stop_propagation();
                    on_delete.call(id_for_delete.clone());
                },
                svg { width: "13", height: "13", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "2", stroke_linecap: "round", stroke_linejoin: "round",
                    path { d: "M3 6h18" }
                    path { d: "M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" }
                    path { d: "M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6" }
                    path { d: "M10 11v6" }
                    path { d: "M14 11v6" }
                }
            }
            // 时间放最右
            span { class: "text-[10px] text-muted font-mono shrink-0", "{session.last_active}" }
        }
    }
}
