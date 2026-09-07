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
        div { class: "floating-task-window",
            div { class: "floating-task-header",
                div { class: "task-header-left",
                    div { class: "floating-task-title", "任务看板" }
                    span { class: "task-progress-badge", "{done_count}/{total_count} 完成 ({pct}%)" }
                }
                button {
                    class: "floating-task-close",
                    onclick: move |_| on_close.call(()),
                    "✕"
                }
            }
            div { class: "task-progress-bar-track",
                div { class: "task-progress-bar-fill", style: "width: {pct}%;" }
            }

            div { class: "floating-task-content",
                div { class: "task-filter-group",
                    button {
                        class: if selected_filter() == "all" { "task-filter-pill active" } else { "task-filter-pill" },
                        onclick: move |_| selected_filter.set("all".into()),
                        "全部 {total_count}"
                    }
                    button {
                        class: if selected_filter() == "in_progress" { "task-filter-pill active" } else { "task-filter-pill" },
                        onclick: move |_| selected_filter.set("in_progress".into()),
                        "进行中 {in_prog_count}"
                    }
                    button {
                        class: if selected_filter() == "open" { "task-filter-pill active" } else { "task-filter-pill" },
                        onclick: move |_| selected_filter.set("open".into()),
                        "待办 {open_count}"
                    }
                    button {
                        class: if selected_filter() == "done" { "task-filter-pill active" } else { "task-filter-pill" },
                        onclick: move |_| selected_filter.set("done".into()),
                        "已完成 {done_count}"
                    }
                    if blocked_count > 0 {
                        button {
                            class: if selected_filter() == "blocked" { "task-filter-pill active" } else { "task-filter-pill" },
                            onclick: move |_| selected_filter.set("blocked".into()),
                            "阻塞 {blocked_count}"
                        }
                    }
                }

                div { class: "progress-task-list",
                    for task in filtered_tasks {
                        {
                            let is_selected = selected_task_id() == Some(task.id.clone());
                            let (status_label, status_chip_class) = match task.status.as_str() {
                                "in_progress" => ("进行中", "task-chip in_progress"),
                                "done" => ("已完成", "task-chip done"),
                                "blocked" => ("已阻塞", "task-chip blocked"),
                                _ => ("待办", "task-chip open"),
                            };
                            let priority_class = match task.priority {
                                0 => "task-p-badge p0",
                                1 => "task-p-badge p1",
                                _ => "task-p-badge p2",
                            };
                            let kind_class = if task.kind == "bug" { "task-kind-chip bug" } else { "task-kind-chip feature" };
                            let task_id = task.id.clone();

                            rsx! {
                                div {
                                    key: "{task.id}",
                                    class: if is_selected { "progress-task-item selected" } else { "progress-task-item" },
                                    onclick: move |_| {
                                        if is_selected {
                                            selected_task_id.set(None);
                                        } else {
                                            selected_task_id.set(Some(task_id.clone()));
                                        }
                                    },
                                    div { class: "task-card-header",
                                        div { class: "task-card-header-left",
                                            span { class: "{status_chip_class}", "{status_label}" }
                                            span { class: "task-id-badge", "#{task.id}" }
                                        }
                                        div { class: "task-card-header-right",
                                            span { class: "{kind_class}", "{task.kind}" }
                                            span { class: "{priority_class}", "P{task.priority}" }
                                        }
                                    }
                                    div { class: "task-card-title", "{task.title}" }
                                    if is_selected {
                                        div { class: "task-card-expanded",
                                            if !task.description.is_empty() {
                                                div { class: "task-detail-block",
                                                    span { class: "task-detail-label", "描述" }
                                                    p { class: "task-detail-text", "{task.description}" }
                                                }
                                            }
                                            if !task.acceptance.is_empty() {
                                                div { class: "task-detail-block",
                                                    span { class: "task-detail-label", "验收标准" }
                                                    p { class: "task-detail-text acceptance", "{task.acceptance}" }
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
