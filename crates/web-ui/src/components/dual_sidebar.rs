//! 双栏侧边栏（ui-kit DualPaneNav 形态）：一级 = 会话 / 设置 / 统计，
//! 二级菜单选项映射中栏页面：
//! - 会话二级 = 自定义 content 槽（项目 → 会话两级折叠树 + ⌘K 搜索小按钮）；
//! - 设置二级 = 默认列表（模型与渠道 / MCP 服务器 / 关于），点击切设置页分区；
//! - 统计 = 一级叶子（点击切统计页；时间范围由页内胶囊选，无二级清单）。
//! 折叠态 = 一级 icon rail（hover 浮出二级 popover，kit 内置）。

use std::collections::{HashMap, HashSet};

use dioxus::prelude::*;
use web_state::types::{Session, WorkspaceSpace};

use crate::nav::{SettingsSection, View};
use crate::shared as sh;

use ui_kit::button::{Button, ButtonSize, ButtonVariant};
use ui_kit::icons::{
    ANIM_SCOPE, IconBox, IconChartBar, IconFolder, IconMessageSquare, IconPlus, IconSearch,
    IconSettings, IconTrash,
};
use ui_kit::layout::{DualPaneChild, DualPaneGroup, DualPaneNav};

use super::session_row::SessionRow;

/// 「会话」二级菜单 content：项目 → 会话两级折叠树（项目行 h34 / 会话行
/// 缩进 22px，markup 与原单栏侧栏同源）+ ⌘K 搜索小按钮（「+ 创建项目 / 归档」属 Batch 3）。
/// 折叠态（rail-only）装不下动态会话数据（DualPaneChild 是 `&'static str`，
/// 会话 id/标题是动态的），故会话组不用默认列表，content 专属展开态。
#[component]
pub fn SessionTreePanel(
    spaces: Vec<WorkspaceSpace>,
    space_sessions: HashMap<String, Vec<Session>>,
    active_id: String,
    on_select: EventHandler<String>,
    on_select_space: EventHandler<String>,
    on_create: EventHandler<String>,
    on_archive_session: EventHandler<String>,
    on_delete_space: EventHandler<String>,
    on_open_search: EventHandler<()>,
    on_create_project: EventHandler<()>,
) -> Element {
    // 项目默认全部展开
    let mut expanded_spaces = use_signal(|| {
        spaces
            .iter()
            .map(|s| s.path.clone())
            .collect::<HashSet<_>>()
    });

    rsx! {
        div { class: "flex flex-col gap-1.5 min-h-0 flex-1",
            // 小按钮行：⌘K 搜索 + 「+ 创建项目」目录弹窗（A2）
            div { class: "flex items-center gap-1.5",
                Button {
                    variant: ButtonVariant::Ghost,
                    size: ButtonSize::IconSm,
                    class: "{ANIM_SCOPE} nav-search-bar",
                    title: sh::MSG_SEARCH_SESSION,
                    onclick: move |_| on_open_search.call(()),
                    IconSearch { size: 15 }
                }
                Button {
                    variant: ButtonVariant::Ghost,
                    size: ButtonSize::IconSm,
                    class: "{ANIM_SCOPE}",
                    title: sh::BTN_CREATE_PROJECT,
                    onclick: move |_| on_create_project.call(()),
                    IconPlus { size: 15 }
                }
            }
            // 项目/会话树
            div { class: "flex-1 min-h-0 overflow-y-auto no-scrollbar flex flex-col pb-1",
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
                            div { class: "group h-[34px] mx-0 px-2 rounded-lg flex items-center gap-2 hover:bg-muted cursor-pointer transition-colors",
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
                                IconFolder { size: 16, class: "shrink-0 text-muted-foreground" }
                                span { class: "role-hint text-foreground truncate min-w-0 flex-1", "{space.name}" }
                                button {
                                    class: "shrink-0 flex items-center justify-center w-4 h-4 text-muted-foreground hover:text-foreground opacity-40 group-hover:opacity-100 transition-opacity cursor-pointer bg-transparent border-none",
                                    title: sh::BTN_NEW_CHAT_IN_SPACE,
                                    onclick: move |e: MouseEvent| {
                                        e.stop_propagation();
                                        on_create.call(space_path_create.clone());
                                    },
                                    IconPlus { size: 13 }
                                }
                                span { class: "role-caption tabular-nums", "{count}" }
                                // 移除项目钮：hover 行才出现，排在行最右
                                button {
                                    class: "shrink-0 flex items-center justify-center w-4 h-4 text-muted-foreground hover:text-destructive opacity-40 group-hover:opacity-100 transition-opacity cursor-pointer bg-transparent border-none",
                                    title: sh::BTN_REMOVE_SPACE,
                                    onclick: move |e: MouseEvent| {
                                        e.stop_propagation();
                                        on_delete_space.call(space_path_delete.clone());
                                    },
                                    IconTrash { size: 13 }
                                }
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
                                            on_archive: on_archive_session,
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
}

/// 双栏侧边栏：ui-kit `DualPaneNav`（左 icon rail + 右二级列表 + 折叠 hover
/// popover + footer 槽）装 kymido 三个一级组。一级点击 = 切中栏页面；
/// 二级选项点击 = 页内定位（设置分区 / 统计范围）。
#[component]
pub fn DualSidebar(
    spaces: Vec<WorkspaceSpace>,
    space_sessions: HashMap<String, Vec<Session>>,
    active_id: String,
    on_select: EventHandler<String>,
    on_select_space: EventHandler<String>,
    on_create: EventHandler<String>,
    on_archive_session: EventHandler<String>,
    on_delete_space: EventHandler<String>,
    on_open_search: EventHandler<()>,
    on_create_project: EventHandler<()>,
    view: Signal<View>,
    settings_section: Signal<SettingsSection>,
    expanded: bool,
) -> Element {
    let on_group_change = move |i: usize| {
        // 一级 = 页面导航（二级选项在该页内细化，点击一级不清页内定位）
        view.set(match i {
            0 => View::Chat,
            1 => View::Settings,
            2 => View::Stats,
            _ => View::Archive,
        });
    };

    let on_child_click = move |id: String| {
        if let Some(sec) = SettingsSection::from_id(&id) {
            settings_section.set(sec);
            view.set(View::Settings);
        }
    };

    let groups = vec![
        DualPaneGroup {
            id: "sessions",
            label: sh::LBL_SESSIONS,
            icon: rsx! { IconMessageSquare { size: 20 } },
            // 动态会话数据装不进 &'static 的 DualPaneChild：会话组不用默认
            // 列表（折叠 popover 只剩组标题），二级内容走 content 槽。
            children: Vec::new(),
            content: Some(rsx! {
                SessionTreePanel {
                    spaces: spaces.clone(),
                    space_sessions: space_sessions.clone(),
                    active_id: active_id.clone(),
                    on_select: on_select,
                    on_select_space: on_select_space,
                    on_create: on_create,
                    on_archive_session: on_archive_session,
                    on_delete_space: on_delete_space,
                    on_open_search: on_open_search,
                    on_create_project: on_create_project,
                }
            }),
        },
        DualPaneGroup {
            id: "settings",
            label: sh::LBL_SETTINGS,
            icon: rsx! { IconSettings { size: 20 } },
            children: vec![
                DualPaneChild {
                    id: "models",
                    label: sh::LBL_MODEL_CHANNEL,
                },
                DualPaneChild {
                    id: "mcp",
                    label: sh::LBL_MCP_SERVERS,
                },
                DualPaneChild {
                    id: "about",
                    label: sh::LBL_ABOUT,
                },
            ],
            content: None,
        },
        DualPaneGroup {
            id: "stats",
            label: sh::TTL_STATS,
            icon: rsx! { IconChartBar { size: 20 } },
            children: Vec::new(),
            content: None,
        },
        // 归档：软删会话的一级 tab（列表在中心页 ArchiveView，同统计/设置页模式）。
        DualPaneGroup {
            id: "archive",
            label: sh::LBL_ARCHIVE,
            icon: rsx! { IconBox { size: 20 } },
            children: Vec::new(),
            content: None,
        },
    ];

    rsx! {
        DualPaneNav {
            groups: groups,
            default_group: 0,
            expanded: expanded,
            on_group_change: on_group_change,
            on_child_click: on_child_click,
            footer: None,
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
pub(crate) fn depth_indent_class(depth: usize) -> &'static str {
    match depth {
        0 => "",
        1 => "pl-[18px]",
        2 => "pl-[36px]",
        _ => "pl-[54px]",
    }
}
