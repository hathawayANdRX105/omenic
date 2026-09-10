use crate::mock::TaskItem;
use dioxus::prelude::*;

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

    let filtered_tasks: Vec<_> = tasks
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
        div { class: "absolute right-4 bottom-4 w-[420px] max-h-[80vh] bg-surface border border-subtle rounded-xl shadow-[0_12px_40px_rgba(0,0,0,0.5)] flex flex-col overflow-hidden z-40",
            div { class: "flex items-center justify-between px-3.5 py-2.5 border-b border-subtle",
                div { class: "flex items-center gap-2",
                    div { class: "text-[13px] font-semibold text-primary", "任务看板" }
                    span { class: "font-mono text-[10.5px] text-muted", "{done_count}/{total_count} 完成 ({pct}%)" }
                }
                button {
                    class: "ml-auto text-muted hover:text-primary cursor-pointer text-[14px] leading-none px-1.5 py-0.5 rounded hover:bg-hover transition-colors",
                    onclick: move |_| on_close.call(()),
                    "✕"
                }
            }
            div { class: "h-1.5 rounded-full bg-base overflow-hidden",
                div { class: "h-full rounded-full bg-accent transition-all", style: "width: {pct}%;" }
            }

            div { class: "flex-1 min-h-0 overflow-y-auto flex flex-col",
                div { class: "flex flex-wrap gap-1.5 px-3.5 py-2.5 border-b border-subtle",
                    button {
                        class: if selected_filter() == "all" { "px-2.5 py-1 rounded-md text-[11px] cursor-pointer text-primary bg-surface-elevated border border-accent transition-colors" } else { "px-2.5 py-1 rounded-md text-[11px] cursor-pointer text-secondary bg-base border border-subtle hover:border-hover transition-colors" },
                        onclick: move |_| selected_filter.set("all".into()),
                        "全部 {total_count}"
                    }
                    button {
                        class: if selected_filter() == "in_progress" { "px-2.5 py-1 rounded-md text-[11px] cursor-pointer text-primary bg-surface-elevated border border-accent transition-colors" } else { "px-2.5 py-1 rounded-md text-[11px] cursor-pointer text-secondary bg-base border border-subtle hover:border-hover transition-colors" },
                        onclick: move |_| selected_filter.set("in_progress".into()),
                        "进行中 {in_prog_count}"
                    }
                    button {
                        class: if selected_filter() == "open" { "px-2.5 py-1 rounded-md text-[11px] cursor-pointer text-primary bg-surface-elevated border border-accent transition-colors" } else { "px-2.5 py-1 rounded-md text-[11px] cursor-pointer text-secondary bg-base border border-subtle hover:border-hover transition-colors" },
                        onclick: move |_| selected_filter.set("open".into()),
                        "待办 {open_count}"
                    }
                    button {
                        class: if selected_filter() == "done" { "px-2.5 py-1 rounded-md text-[11px] cursor-pointer text-primary bg-surface-elevated border border-accent transition-colors" } else { "px-2.5 py-1 rounded-md text-[11px] cursor-pointer text-secondary bg-base border border-subtle hover:border-hover transition-colors" },
                        onclick: move |_| selected_filter.set("done".into()),
                        "已完成 {done_count}"
                    }
                    if blocked_count > 0 {
                        button {
                            class: if selected_filter() == "blocked" { "px-2.5 py-1 rounded-md text-[11px] cursor-pointer text-primary bg-surface-elevated border border-accent transition-colors" } else { "px-2.5 py-1 rounded-md text-[11px] cursor-pointer text-secondary bg-base border border-subtle hover:border-hover transition-colors" },
                            onclick: move |_| selected_filter.set("blocked".into()),
                            "阻塞 {blocked_count}"
                        }
                    }
                }

                div { class: "flex flex-col gap-1.5 p-3 overflow-y-auto",
                    for task in filtered_tasks {
                        {
                            let is_selected = selected_task_id() == Some(task.id.clone());
                            let (status_label, status_chip_class) = match task.status.as_str() {
                                "in_progress" => ("进行中", "px-1.5 py-0.5 rounded text-[10px] font-medium bg-[rgba(162,138,199,0.15)] text-accent border border-[rgba(162,138,199,0.3)]"),
                                "done" => ("已完成", "px-1.5 py-0.5 rounded text-[10px] font-medium bg-emerald-500/10 text-success border border-emerald-500/30"),
                                "blocked" => ("已阻塞", "px-1.5 py-0.5 rounded text-[10px] font-medium bg-red-500/10 text-danger border border-red-500/30"),
                                _ => ("待办", "px-1.5 py-0.5 rounded text-[10px] font-medium bg-[rgba(255,255,255,0.06)] text-muted border border-[rgba(255,255,255,0.1)]"),
                            };
                            let priority_class = match task.priority {
                                0 => "px-1.5 py-0.5 rounded text-[10px] font-mono font-bold bg-red-500/10 text-danger",
                                1 => "px-1.5 py-0.5 rounded text-[10px] font-mono font-bold bg-amber-500/10 text-amber-300",
                                _ => "px-1.5 py-0.5 rounded text-[10px] font-mono font-bold bg-[rgba(255,255,255,0.06)] text-muted",
                            };
                            let kind_class = if task.kind == "bug" { "px-1.5 py-0.5 rounded text-[10px] font-mono uppercase font-bold bg-red-500/10 text-danger" } else { "px-1.5 py-0.5 rounded text-[10px] font-mono uppercase font-bold bg-violet-500/10 text-violet-300" };
                            let task_id = task.id.clone();

                            rsx! {
                                div {
                                    key: "{task.id}",
                                    class: if is_selected { "rounded-lg border border-accent bg-surface-elevated px-3 py-2.5 cursor-pointer transition-colors" } else { "rounded-lg border border-subtle bg-base px-3 py-2.5 cursor-pointer transition-colors hover:border-hover" },
                                    onclick: move |_| {
                                        if is_selected {
                                            selected_task_id.set(None);
                                        } else {
                                            selected_task_id.set(Some(task_id.clone()));
                                        }
                                    },
                                    div { class: "flex items-center justify-between mb-1.5",
                                        div { class: "flex items-center gap-1.5",
                                            span { class: "{status_chip_class}", "{status_label}" }
                                            span { class: "font-mono text-[10px] text-muted", "#{task.id}" }
                                        }
                                        div { class: "flex items-center gap-1.5",
                                            span { class: "{kind_class}", "{task.kind}" }
                                            span { class: "{priority_class}", "P{task.priority}" }
                                        }
                                    }
                                    div { class: "text-[13px] font-medium text-primary", "{task.title}" }
                                    if is_selected {
                                        div { class: "mt-2 pt-2 border-t border-subtle flex flex-col gap-2",
                                            if !task.description.is_empty() {
                                                div { class: "flex flex-col gap-0.5",
                                                    span { class: "text-[10px] uppercase tracking-wide text-muted font-semibold", "描述" }
                                                    p { class: "text-[12px] text-secondary leading-relaxed", "{task.description}" }
                                                }
                                            }
                                            if !task.acceptance.is_empty() {
                                                div { class: "flex flex-col gap-0.5",
                                                    span { class: "text-[10px] uppercase tracking-wide text-muted font-semibold", "验收标准" }
                                                    p { class: "text-[12px] text-secondary leading-relaxed", "{task.acceptance}" }
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
}
