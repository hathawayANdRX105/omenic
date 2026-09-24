//! 侧栏（dsh SidebarRoot 复刻）：logo 行 + 新会话钮 + 项目/会话树 +
//! 底部「数据统计 / 设置」触发行；折叠 rail 56px。

use dioxus::prelude::*;
use std::collections::{HashMap, HashSet};
use web_state::types::{Session, SessionStatus, WorkspaceSpace};

use crate::icons::{Chart, Folder, Gear, PanelLeft, Plus, Search, Trash};
use crate::ui::IconButton;

fn status_dot_class(status: &SessionStatus) -> &'static str {
    match status {
        SessionStatus::Active => "bg-brand",
        SessionStatus::Idle => "bg-dim",
        // 中断/半开 run 与归档同走 danger 色（WP-C）
        SessionStatus::Archived | SessionStatus::Aborted => "bg-danger/70",
    }
}

#[component]
pub fn Sidebar(
    spaces: Vec<WorkspaceSpace>,
    space_sessions: HashMap<String, Vec<Session>>,
    active_id: String,
    on_select: EventHandler<String>,
    on_select_space: EventHandler<String>,
    on_create: EventHandler<String>,
    /// 行内「+」钮：以某会话为父创建子会话（5.3/5.5 谱系分组）
    on_create_child: EventHandler<String>,
    on_delete_session: EventHandler<String>,
    on_delete_space: EventHandler<String>,
    collapsed: bool,
    on_toggle: EventHandler<()>,
    width: usize,
    on_resize_start: EventHandler<i32>,
    on_preset_start: EventHandler<i32>,
    on_expand: EventHandler<()>,
    on_open_search: EventHandler<()>,
    on_open_stats: EventHandler<()>,
    on_open_settings: EventHandler<()>,
) -> Element {
    // 项目默认全部展开
    let mut expanded_spaces = use_signal(|| {
        spaces
            .iter()
            .map(|s| s.path.clone())
            .collect::<HashSet<_>>()
    });

    // 多个 move 闭包要读「第一个项目」——提前取好，各自克隆
    let first_space_path = spaces.first().map(|s| s.path.clone());
    let first_path_logo = first_space_path.clone();
    let first_path_new_chat = first_space_path;

    let aside_style = if collapsed {
        "width:56px;min-width:56px;".to_string()
    } else {
        format!("width:{width}px;min-width:{width}px;")
    };

    rsx! {
        aside { class: "relative h-full bg-sidebar border-r border-b1 flex flex-col shrink-0 select-none",
            style: "{aside_style}",
            if collapsed {
                CollapsedRail {
                    spaces: spaces.clone(),
                    space_sessions: space_sessions.clone(),
                    active_id: active_id.clone(),
                    on_select: on_select,
                    on_toggle: on_toggle,
                    on_expand: on_expand,
                    on_open_stats: on_open_stats,
                    on_open_settings: on_open_settings,
                    on_preset_start: on_preset_start,
                }
            } else {
                div { class: "flex-1 min-h-0 flex flex-col px-3 pt-1.5",
                    // Logo 行：品牌字标 + 折叠钮
                    div { class: "h-[52px] flex items-center justify-between pl-1 pr-0",
                        button {
                            class: "cursor-pointer bg-transparent border-none p-0",
                            title: "新会话",
                            onclick: move |_| {
                                if let Some(path) = first_path_logo.clone() {
                                    on_create.call(path);
                                }
                            },
                            crate::icons::Wordmark {}
                        }
                        IconButton { title: "收起侧边栏", onclick: move |_| on_toggle.call(()), PanelLeft { size: 16 } }
                    }
                    // 新会话按钮：h38 r12 elevated + 边框
                    button {
                        class: "h-[38px] mx-0.5 mb-2 px-4 rounded-xl border border-b2 bg-layer-2 hover:bg-layer-3 hover:border-b3 flex items-center justify-center gap-1.5 text-[14px] leading-[22px] text-label-2 hover:text-label transition-colors cursor-pointer",
                        onclick: move |_| {
                            if let Some(path) = first_path_new_chat.clone() {
                                on_create.call(path);
                            }
                        },
                        Plus { size: 15, class: "text-label-3" }
                        span { "新会话" }
                    }
                    // 区头：标题 + 搜索（⌘K 快速切换入口）
                    div { class: "h-9 flex items-center justify-between pl-1 pr-0.5",
                        span { class: "text-[12px] leading-4 text-caption", "会话" }
                        IconButton {
                            class: "nav-search-bar",
                            title: "搜索会话 (⌘K)",
                            onclick: move |_| on_open_search.call(()),
                            Search { size: 15 }
                        }
                    }
                    // 项目/会话树
                    div { class: "flex-1 min-h-0 overflow-y-auto flex flex-col pb-2",
                        for space in spaces {
                            {
                                let space_path = space.path.clone();
                                let space_path_create = space.path.clone();
                                let space_path_delete = space.path.clone();
                                let space_path_toggle = space.path.clone();
                                let is_open = expanded_spaces().contains(&space_path);
                                let sessions_for_space = space_sessions
                                    .get(&space.path)
                                    .cloned()
                                    .unwrap_or_default();
                                let count = sessions_for_space.len();
                                rsx! {
                                    // 项目行 h34
                                    div { class: "group h-[34px] mx-0 px-2 rounded-lg flex items-center gap-2 hover:bg-ihover cursor-pointer transition-colors",
                                        onclick: move |_| {
                                            let mut set = expanded_spaces.write();
                                            if set.contains(&space_path_toggle) {
                                                set.remove(&space_path_toggle);
                                            } else {
                                                set.insert(space_path_toggle.clone());
                                            }
                                            drop(set);
                                            on_select_space.call(space_path.clone());
                                        },
                                        Folder { size: 16, class: "shrink-0 text-label-3" }
                                        span { class: "text-[14px] leading-5 text-label truncate min-w-0 flex-1", "{space.name}" }
                                        button {
                                            class: "shrink-0 flex items-center justify-center w-4 h-4 text-label-3 hover:text-label opacity-0 group-hover:opacity-100 transition-opacity cursor-pointer bg-transparent border-none",
                                            title: "新建会话",
                                            onclick: move |e: MouseEvent| {
                                                e.stop_propagation();
                                                on_create.call(space_path_create.clone());
                                            },
                                            Plus { size: 13 }
                                        }
                                        button {
                                            class: "shrink-0 flex items-center justify-center w-4 h-4 text-label-3 hover:text-danger opacity-0 group-hover:opacity-100 transition-opacity cursor-pointer bg-transparent border-none",
                                            title: "移除项目",
                                            onclick: move |e: MouseEvent| {
                                                e.stop_propagation();
                                                on_delete_space.call(space_path_delete.clone());
                                            },
                                            Trash { size: 13 }
                                        }
                                        span { class: "text-[12px] leading-5 text-label-3 tabular-nums", "{count}" }
                                    }
                                    // 会话行 h32，缩进 22px；树内子会话再逐层缩进
                                    if is_open {
                                        div { class: "pl-[22px] flex flex-col gap-px pb-1",
                                            for (session, depth) in group_sessions(&sessions_for_space) {
                                                SessionRow {
                                                    key: "{session.id}",
                                                    session: session.clone(),
                                                    depth,
                                                    active: session.id == active_id,
                                                    on_select: on_select,
                                                    on_create_child: on_create_child,
                                                    on_delete: on_delete_session,
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                // 底部：数据统计 / 设置
                div { class: "px-3 pb-2 flex flex-col gap-0.5",
                    div { class: "h-[42px] px-2.5 rounded-xl flex items-center gap-2.5 text-[14px] leading-[22px] text-label-2 hover:bg-ihover hover:text-label cursor-pointer transition-colors",
                        onclick: move |_| on_open_stats.call(()),
                        Chart { size: 16, class: "text-label-3" }
                        span { "数据统计" }
                    }
                    div { class: "h-[42px] px-2.5 rounded-xl flex items-center gap-2.5 text-[14px] leading-[22px] text-label-2 hover:bg-ihover hover:text-label cursor-pointer transition-colors",
                        onclick: move |_| on_open_settings.call(()),
                        Gear { size: 16, class: "text-label-3" }
                        span { "设置" }
                    }
                }
                // 右缘拖拽把手
                div {
                    class: "absolute top-0 right-0 h-full w-1.5 cursor-col-resize hover:bg-brand/40 transition-colors",
                    onmousedown: move |e: MouseEvent| on_resize_start.call(e.client_coordinates().x as i32),
                }
            }
        }
    }
}

/// 把一个 space 内的扁平会话列表按 `parent_id` 组成渲染树：父在前、子紧随
/// 其后并逐层加深，返回 `(会话引用, 层级)`。`parent_id` 是无 FK 的自由文本，
/// 三条边界：父不在本列表（被删 / 在别的 space）→ 当根；成环（a→b→a）
/// 由 visited 集合截断并在第二趟当根兜底；层级超深时缩进封顶但**节点照常
/// 渲染**——深链不该让会话从侧栏消失。绝不死循环。渲染顺序沿原列表顺序，
/// 与 page-workspace「子插在父之后」对齐。
///
/// `pub` 是为了让 `tests/group_sessions.rs` 锁住这几条边界不变式——它跑在
/// 渲染路径上，一旦死循环或层级算错，表现是侧栏卡死/错位而不是编译失败。
pub fn group_sessions(sessions: &[Session]) -> Vec<(&Session, usize)> {
    const MAX_DEPTH: usize = 16;

    let by_id: HashMap<&str, &Session> = sessions.iter().map(|s| (s.id.as_str(), s)).collect();
    // 只认父也在本列表内的边；父不在 → 该会话是根
    let mut children: HashMap<&str, Vec<&Session>> = HashMap::new();
    for s in sessions {
        if let Some(pid) = s.parent_id.as_deref()
            && by_id.contains_key(pid)
        {
            children.entry(pid).or_default().push(s);
        }
    }

    let mut visited: HashSet<&str> = HashSet::new();
    let mut out: Vec<(&Session, usize)> = Vec::new();
    let is_root = |s: &Session| match s.parent_id.as_deref() {
        Some(pid) => !by_id.contains_key(pid),
        None => true,
    };
    // 两趟：第一趟只处理根，第二趟兜底成环残余（a↔b 互相指向时二者都不是根）
    for pass in 0..2u8 {
        for s in sessions {
            if visited.contains(s.id.as_str()) {
                continue;
            }
            if pass == 0 && !is_root(s) {
                continue;
            }
            let mut stack: Vec<(&Session, usize)> = vec![(s, 0)];
            while let Some((node, depth)) = stack.pop() {
                if !visited.insert(node.id.as_str()) {
                    continue;
                }
                out.push((node, depth));
                if let Some(kids) = children.get(node.id.as_str()) {
                    // 反序入栈，弹出时保持原列表顺序。超过 MAX_DEPTH 的后代
                    // 仍然渲染，只是缩进封顶——深链不该让会话从侧栏消失。
                    for child in kids.iter().rev() {
                        stack.push((child, (depth + 1).min(MAX_DEPTH)));
                    }
                }
            }
        }
    }
    out
}

/// 子会话逐层缩进 18px；根会话由外层容器的 `pl-[22px]` 统一缩进，故
/// depth 0 不再叠加。层级封顶 3 层，更深的嵌套不再加宽，避免把行内容
/// 挤出侧栏可视区。
fn depth_indent_class(depth: usize) -> &'static str {
    match depth {
        0 => "",
        1 => "pl-[18px]",
        2 => "pl-[36px]",
        _ => "pl-[54px]",
    }
}

#[component]
fn SessionRow(
    session: Session,
    depth: usize,
    active: bool,
    on_select: EventHandler<String>,
    on_create_child: EventHandler<String>,
    on_delete: EventHandler<String>,
) -> Element {
    let id_for_select = session.id.clone();
    let id_for_child = session.id.clone();
    let id_for_delete = session.id.clone();
    let dot = status_dot_class(&session.status);

    let row_class = if active {
        "group h-8 px-2 rounded-lg flex items-center gap-2 bg-ihover cursor-pointer transition-colors"
    } else {
        "group h-8 px-2 rounded-lg flex items-center gap-2 hover:bg-ihover cursor-pointer transition-colors"
    };
    let title_class = if active {
        "text-[14px] leading-5 text-label truncate min-w-0 flex-1"
    } else {
        "text-[14px] leading-5 text-label-2 truncate min-w-0 flex-1"
    };

    rsx! {
        div { class: "{row_class} {depth_indent_class(depth)}",
            onclick: move |_| on_select.call(id_for_select.clone()),
            span { class: "w-2 h-2 rounded-full {dot} shrink-0" }
            span { class: "{title_class}", "{session.title}" }
            // 新建子会话：样式语言同项目行 + 钮，与删除钮共用 hover 让位
            // 出来的空间（时间戳 group-hover:hidden）
            button {
                class: "shrink-0 flex items-center justify-center w-4 h-4 text-label-3 hover:text-label opacity-0 group-hover:opacity-100 transition-opacity cursor-pointer bg-transparent border-none",
                title: "新建子会话",
                onclick: move |e: MouseEvent| {
                    e.stop_propagation();
                    on_create_child.call(id_for_child.clone());
                },
                Plus { size: 13 }
            }
            button {
                class: "shrink-0 flex items-center justify-center w-4 h-4 text-label-3 hover:text-danger opacity-0 group-hover:opacity-100 transition-opacity cursor-pointer bg-transparent border-none",
                title: "删除会话",
                onclick: move |e: MouseEvent| {
                    e.stop_propagation();
                    on_delete.call(id_for_delete.clone());
                },
                Trash { size: 13 }
            }
            span { class: "text-[12px] leading-5 text-label-3 shrink-0 group-hover:hidden", "{session.last_active}" }
        }
    }
}

/// 折叠 rail（56px）：图标列 + CSS 悬停信息面板。
#[component]
fn CollapsedRail(
    spaces: Vec<WorkspaceSpace>,
    space_sessions: HashMap<String, Vec<Session>>,
    active_id: String,
    on_select: EventHandler<String>,
    on_toggle: EventHandler<()>,
    on_expand: EventHandler<()>,
    on_open_stats: EventHandler<()>,
    on_open_settings: EventHandler<()>,
    on_preset_start: EventHandler<i32>,
) -> Element {
    rsx! {
        div { class: "pt-3 pb-2 px-[10px] flex flex-col items-center gap-1.5 h-full",
            button {
                class: "w-9 h-9 flex items-center justify-center rounded-full text-label-3 hover:bg-ihover hover:text-label-2 transition-colors cursor-pointer bg-transparent border-none",
                title: "展开侧边栏",
                onclick: move |_| on_toggle.call(()),
                PanelLeft { size: 16 }
            }
            button {
                class: "w-9 h-9 flex items-center justify-center rounded-full border border-b2 text-label-3 hover:bg-ihover hover:text-label-2 transition-colors cursor-pointer bg-transparent",
                title: "新会话",
                onclick: move |_| on_expand.call(()),
                Plus { size: 15 }
            }
            div { class: "my-1 w-5 h-px bg-b2" }
            div { class: "flex-1 min-h-0 overflow-y-auto w-full flex flex-col items-center gap-1",
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
                                    let dot = status_dot_class(&s.status);
                                    rsx! {
                                        div { class: "relative group shrink-0",
                                            button {
                                                class: if is_selected {
                                                    "w-9 h-9 flex items-center justify-center rounded-full cursor-pointer bg-ihover transition-colors bg-transparent border-none"
                                                } else {
                                                    "w-9 h-9 flex items-center justify-center rounded-full cursor-pointer hover:bg-ihover transition-colors bg-transparent border-none"
                                                },
                                                onclick: move |_| on_select.call(sid.clone()),
                                                span { class: "w-2.5 h-2.5 rounded-full {dot}" }
                                            }
                                            // 悬停信息面板（纯 CSS）
                                            div { class: "absolute left-full top-1/2 -translate-y-1/2 ml-2 z-50 w-52 rounded-xl border border-binv bg-menu px-3 py-2 shadow-lv3 opacity-0 pointer-events-none group-hover:opacity-100 transition-opacity duration-150",
                                                div { class: "text-[13px] leading-5 font-medium text-label truncate", "{stitle}" }
                                                div { class: "mt-1 flex items-center justify-between gap-2",
                                                    span { class: "text-[11px] text-label-3 truncate", "{sp_name}" }
                                                    span { class: "text-[11px] text-label-3 font-mono shrink-0", "{slast}" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "flex flex-col items-center gap-1.5",
                div {
                    class: "h-px w-6 bg-b2 cursor-ew-resize hover:bg-brand",
                    title: "拖拽设定默认宽度",
                    onmousedown: move |e: MouseEvent| on_preset_start.call(e.client_coordinates().x as i32),
                }
                button {
                    class: "w-9 h-9 flex items-center justify-center rounded-full text-label-3 hover:bg-ihover hover:text-label-2 transition-colors cursor-pointer bg-transparent border-none",
                    title: "数据统计",
                    onclick: move |_| on_open_stats.call(()),
                    Chart { size: 16 }
                }
                button {
                    class: "w-9 h-9 flex items-center justify-center rounded-full text-label-3 hover:bg-ihover hover:text-label-2 transition-colors cursor-pointer bg-transparent border-none",
                    title: "设置",
                    onclick: move |_| on_open_settings.call(()),
                    Gear { size: 16 }
                }
            }
        }
    }
}
