//! 任务 dock 卡片（dsh composer 上方 dock 形态）：宽随消息列，r16 浮层。

use dioxus::prelude::*;
use web_state::types::TaskItem;

use crate::components::icons::X;
use crate::components::ui::IconButton;

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
        div { class: "w-full rounded-2xl border border-binv bg-layer-2 shadow-lv2 overflow-hidden pointer-events-auto",
            // 头部
            div { class: "flex items-center justify-between pl-4 pr-2 py-1.5 border-b border-b1",
                div { class: "flex items-center gap-2.5",
                    span { class: "text-[14px] leading-5 font-medium text-label", "任务看板" }
                    span { class: "text-[12px] leading-5 text-label-3", "{done_count}/{total_count} 完成" }
                }
                IconButton { title: "关闭", onclick: move |_| on_close.call(()), X { size: 14 } }
            }
            // 进度条
            div { class: "h-1 bg-layer-3",
                div { class: "h-full bg-brand transition-all", style: "width: {pct}%;" }
            }
            // 过滤 chips（dsh compact：h26 r7）
            div { class: "flex flex-wrap gap-1 px-3 py-2 border-b border-b1",
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
                            "in_progress" => ("进行中", "bg-chip-brand text-brand-300"),
                            "done" => ("已完成", "bg-chip-success text-success-2"),
                            "blocked" => ("已阻塞", "bg-chip-danger text-danger"),
                            _ => ("待办", "bg-layer-2 text-label-3"),
                        };
                        let priority_class = match task.priority {
                            0 => "bg-chip-danger text-danger",
                            1 => "bg-chip-warn text-warn-2",
                            _ => "bg-layer-2 text-label-3",
                        };
                        let card_class = if is_selected {
                            "rounded-[10px] border border-brand bg-layer-1 px-3 py-2.5 cursor-pointer transition-colors"
                        } else {
                            "rounded-[10px] border border-b1 bg-layer-1 px-3 py-2.5 cursor-pointer transition-colors hover:border-b2"
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
                                        span { class: "{status_chip} px-1.5 py-px rounded-md text-[10px] leading-4 font-medium", "{status_label}" }
                                        span { class: "font-mono text-[10px] leading-4 text-caption", "#{task.id}" }
                                    }
                                    div { class: "flex items-center gap-1.5",
                                        span { class: "font-mono text-[10px] leading-4 uppercase text-label-3", "{task.kind}" }
                                        span { class: "{priority_class} px-1.5 py-px rounded-md font-mono text-[10px] leading-4 font-semibold", "P{task.priority}" }
                                    }
                                }
                                div { class: "text-[14px] leading-5 text-label", "{task.title}" }
                                if is_selected {
                                    div { class: "mt-2 pt-2 border-t border-b1 flex flex-col gap-1.5",
                                        if !task.description.is_empty() {
                                            div { class: "flex flex-col gap-0.5",
                                                span { class: "text-[10px] leading-4 uppercase tracking-wide text-caption font-semibold", "描述" }
                                                p { class: "text-[12px] leading-[18px] text-label-2", "{task.description}" }
                                            }
                                        }
                                        if !task.acceptance.is_empty() {
                                            div { class: "flex flex-col gap-0.5",
                                                span { class: "text-[10px] leading-4 uppercase tracking-wide text-caption font-semibold", "验收标准" }
                                                p { class: "text-[12px] leading-[18px] text-label-2", "{task.acceptance}" }
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

#[component]
fn FilterChip(label: String, active: bool, onclick: EventHandler<MouseEvent>) -> Element {
    let class = if active {
        "h-[26px] px-[7px] rounded-[7px] text-[12px] leading-[18px] bg-ihover text-label cursor-pointer transition-colors border-none"
    } else {
        "h-[26px] px-[7px] rounded-[7px] text-[12px] leading-[18px] text-label-3 hover:bg-ihover hover:text-label cursor-pointer transition-colors border-none bg-transparent"
    };
    rsx! {
        button { r#type: "button", class: "{class}", onclick: onclick, "{label}" }
    }
}
