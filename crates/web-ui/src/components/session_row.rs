//! 会话行（自 sidebar.rs 拆出）：tree 内的单个会话行，仅被 Sidebar 使用。

use dioxus::prelude::*;
use web_state::types::{Session, SessionStatus};

use crate::shared as sh;
use ui_kit::icons::IconTrash;

use super::dual_sidebar::depth_indent_class;

#[component]
pub(crate) fn SessionRow(
    session: Session,
    depth: usize,
    active: bool,
    on_select: EventHandler<String>,
    on_archive: EventHandler<String>,
) -> Element {
    let id_for_select = session.id.clone();
    let id_for_archive = session.id.clone();
    // 运行中（事件流实时写入 Active）→ 行左侧旋转指示
    let running = session.status == SessionStatus::Active;

    let row_class = if active {
        "group h-8 px-2 rounded-lg flex items-center gap-2 bg-accent cursor-pointer transition-colors"
    } else {
        "group h-8 px-2 rounded-lg flex items-center gap-2 hover:bg-muted cursor-pointer transition-colors"
    };
    let title_class = if active {
        "role-hint text-foreground truncate min-w-0 flex-1"
    } else {
        "role-hint truncate min-w-0 flex-1"
    };

    rsx! {
        div { class: "{row_class} {depth_indent_class(depth)}",
            onclick: move |_| on_select.call(id_for_select.clone()),
            if running {
                ui_kit::Spinner { size: 12, class: "text-brand shrink-0" }
            }
            span { class: "{title_class}", "{session.title}" }
            span { class: "role-caption shrink-0", "{session.last_active}" }
            // 归档会话钮（软删：移入「归档」而非硬删）：hover 行才出现，排在行最右
            button {
                class: "shrink-0 flex items-center justify-center w-4 h-4 text-muted-foreground hover:text-destructive opacity-0 group-hover:opacity-100 transition-opacity cursor-pointer bg-transparent border-none",
                title: sh::BTN_ARCHIVE_SESSION,
                onclick: move |e: MouseEvent| {
                    e.stop_propagation();
                    on_archive.call(id_for_archive.clone());
                },
                IconTrash { size: 13 }
            }
        }
    }
}
