//! 任务 dock 卡片（dsh composer 上方 dock 形态）：宽随消息列，r16 浮层。

use dioxus::prelude::*;
use web_state::types::TaskItem;

use crate::shared as sh;
use crate::styles as st;
use ui_kit::button::{Button, ButtonSize, ButtonVariant};
use ui_kit::icons::IconX;

use super::filter_chip::FilterChip;

#[component]
pub fn TaskPanel(tasks: Vec<TaskItem>, on_close: EventHandler<()>) -> Element {
    let mut selected_filter = use_signal(|| "all".to_string());
    let mut selected_task_id = use_signal(|| None::<String>);
    let total_count = tasks.len();
    let done_count = tasks.iter().filter(|t| t.status == "done").count();
    let in_prog_count = tasks.iter().filter(|t| t.status == "in_progress").count();
    let open_count = tasks.iter().filter(|t| t.status == "open").count();
    let blocked_count = tasks.iter().filter(|t| t.status == "blocked").count();
    let pct = if total_count > 0 {
        (done_count as f64 / total_count as f64 * 100.0).round() as u32
    } else {
        0
    };

    let filtered_tasks: Vec<&TaskItem> = tasks
        .iter()
        .filter(|t| match selected_filter().as_str() {
            "all" => true,
            "in_progress" => t.status == "in_progress",
            "open" => t.status == "open",
            "done" => t.status == "done",
            "blocked" => t.status == "blocked",
            _ => true,
        })
        .collect();

    rsx! {
        // 面板内部点击统一 stop_propagation：不冒泡到 chat 根节点的
        // 「点外关闭」（ainnotation 波3 #5），面板内交互（filter chip /
        // 任务卡展开 / 关闭钮）不误关自身。
        div { class: "w-full rounded-2xl border border-binv bg-secondary shadow-lv2 overflow-hidden pointer-events-auto",
            onclick: move |e: MouseEvent| e.stop_propagation(),
            // 头部
            div { class: "flex items-center justify-between pl-4 pr-2 py-1.5 border-b border-border",
                div { class: "flex items-center gap-2.5",
                    span { class: "role-hint font-medium text-foreground", {sh::BTN_TASK_PANEL} }
                    span { class: "role-caption", "{done_count}/{total_count} 完成" }
                }
                Button {
                    variant: ButtonVariant::Ghost,
                    size: ButtonSize::IconSm,
                    title: sh::BTN_CLOSE,
                    onclick: move |_| on_close.call(()),
                    IconX { size: 14 }
                }
            }
            // 进度条
            div { class: "h-1 bg-secondary-hover",
                div { class: "h-full bg-brand transition-all", style: "width: {pct}%;" }
            }
            // 过滤 chips（dsh compact：h26 r7）
            div { class: "flex flex-wrap gap-1 px-3 py-2 border-b border-border",
                FilterChip {
                    label: "全部 {total_count}",
                    active: selected_filter() == "all",
                    onclick: move |_| selected_filter.set("all".into()),
                }
                FilterChip {
                    label: "进行中 {in_prog_count}",
                    active: selected_filter() == "in_progress",
                    onclick: move |_| selected_filter.set("in_progress".into()),
                }
                FilterChip {
                    label: "待办 {open_count}",
                    active: selected_filter() == "open",
                    onclick: move |_| selected_filter.set("open".into()),
                }
                FilterChip {
                    label: "已完成 {done_count}",
                    active: selected_filter() == "done",
                    onclick: move |_| selected_filter.set("done".into()),
                }
                if blocked_count > 0 {
                    FilterChip {
                        label: "阻塞 {blocked_count}",
                        active: selected_filter() == "blocked",
                        onclick: move |_| selected_filter.set("blocked".into()),
                    }
                }
            }
            // 列表
            div { class: "max-h-[300px] overflow-y-auto p-2 flex flex-col gap-1.5",
                for task in filtered_tasks {
                    {
                        let is_selected = selected_task_id() == Some(task.id.clone());
                        let (status_label, status_chip) = match task.status.as_str() {
                            "in_progress" => (sh::STATUS_IN_PROGRESS, st::S_CHIP_BRAND),
                            "done" => (sh::STATUS_DONE, st::S_CHIP_SUCCESS),
                            "blocked" => (sh::STATUS_BLOCKED, st::S_CHIP_DANGER),
                            _ => (sh::STATUS_OPEN, st::S_RAISED),
                        };
                        let priority_class = match task.priority {
                            0 => "bg-chip-danger text-destructive",
                            1 => "bg-warning text-warning-foreground",
                            _ => "bg-secondary",
                        };
                        let card_class = if is_selected {
                            "rounded-[10px] border border-brand bg-card px-3 py-2.5 cursor-pointer transition-colors"
                        } else {
                            "rounded-[10px] border border-border bg-card px-3 py-2.5 cursor-pointer transition-colors hover:border-border"
                        };
                        let task_id = task.id.clone();
                        rsx! {
                            div { class: "{card_class}",
                                onclick: move |_| {
                                    if is_selected {
                                        selected_task_id.set(None);
                                    } else {
                                        selected_task_id.set(Some(task_id.clone()));
                                    }
                                },
                                div { class: "flex items-center justify-between mb-1",
                                    div { class: "flex items-center gap-1.5",
                                        span { class: "{status_chip} px-1.5 py-px rounded-md role-label text-foreground font-medium", "{status_label}" }
                                        span { class: "font-mono role-label", "#{task.id}" }
                                    }
                                    div { class: "flex items-center gap-1.5",
                                        span { class: "font-mono role-overline", "{task.kind}" }
                                        span { class: "{priority_class} px-1.5 py-px rounded-md font-mono role-label font-semibold", "P{task.priority}" }
                                    }
                                }
                                div { class: "role-hint text-foreground", "{task.title}" }
                                if is_selected {
                                    div { class: "mt-2 pt-2 border-t border-border flex flex-col gap-1.5",
                                        if !task.description.is_empty() {
                                            div { class: "flex flex-col gap-0.5",
                                                span { class: "role-overline", {sh::LBL_DESCRIPTION} }
                                                p { class: "role-caption", "{task.description}" }
                                            }
                                        }
                                        if !task.acceptance.is_empty() {
                                            div { class: "flex flex-col gap-0.5",
                                                span { class: "role-overline", {sh::LBL_ACCEPTANCE} }
                                                p { class: "role-caption", "{task.acceptance}" }
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
}
