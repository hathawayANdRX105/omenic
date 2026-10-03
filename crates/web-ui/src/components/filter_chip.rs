//! 过滤 chip（自 taskpanel.rs 拆出）：任务看板顶部的小型状态过滤按钮。

use dioxus::prelude::*;

/// 单 FilterChip：小型 tab 按钮，active 时底 bg-accent 高亮。
#[component]
pub(crate) fn FilterChip(
    label: String,
    active: bool,
    onclick: EventHandler<MouseEvent>,
) -> Element {
    let class = if active {
        "h-[26px] px-[7px] rounded-[7px] role-caption bg-accent text-foreground cursor-pointer transition-colors border-none"
    } else {
        "h-[26px] px-[7px] rounded-[7px] role-caption hover:bg-muted hover:text-foreground cursor-pointer transition-colors border-none bg-transparent"
    };
    rsx! {
        button { r#type: "button", class: "{class}", onclick: onclick, "{label}" }
    }
}
