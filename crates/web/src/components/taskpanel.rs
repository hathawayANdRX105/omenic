use crate::mock::TaskItem;
use dioxus::prelude::*;

#[component]
pub fn TaskPanel(tasks: Vec<TaskItem>, on_close: EventHandler<()>) -> Element {
    let mut selected_filter = use_signal(|| "all".to_string());
    let mut selected_task_id = use_signal(|| None::<String>);

    let filtered_tasks: Vec<_> = tasks
        .iter()
        .filter(|t| match selected_filter().as_str() {
            "all" => true,
            "open" => t.status == "open",
            "in_progress" => t.status == "in_progress",
            "blocked" => t.status == "blocked",
            "done" => t.status == "done",
            _ => true,
        })
        .collect();

    rsx! {
        div { class: "floating-task-window",
            div { class: "floating-task-header",
                div { class: "floating-task-title", "TASKS ({tasks.len()})" }
                button {
                    class: "floating-task-close",
                    onclick: move |_| on_close.call(()),
                    "✕"
                }
            }
            div { class: "floating-task-content",
                div { class: "task-filter-group",
                    button {
                        class: if selected_filter() == "all" { "task-filter-pill active" } else { "task-filter-pill" },
                        onclick: move |_| selected_filter.set("all".into()),
                        "全部"
                    }
                    button {
                        class: if selected_filter() == "in_progress" { "task-filter-pill active" } else { "task-filter-pill" },
                        onclick: move |_| selected_filter.set("in_progress".into()),
                        "进行中"
                    }
                    button {
                        class: if selected_filter() == "open" { "task-filter-pill active" } else { "task-filter-pill" },
                        onclick: move |_| selected_filter.set("open".into()),
                        "待办"
                    }
                    button {
                        class: if selected_filter() == "done" { "task-filter-pill active" } else { "task-filter-pill" },
                        onclick: move |_| selected_filter.set("done".into()),
                        "已完成"
                    }
                }
                div { class: "progress-task-list",
                    for task in filtered_tasks {
                        {
                            let is_selected = selected_task_id() == Some(task.id.clone());
                            let dot_class = match task.status.as_str() {
                                "in_progress" => "task-dot in_progress",
                                "done" => "task-dot done",
                                "blocked" => "task-dot blocked",
                                _ => "task-dot",
                            };
                            let priority_str = format!("P{}", task.priority);
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
                                    div { class: "task-row-top",
                                        span { class: "{dot_class}" }
                                        span { class: "task-id", "{task.id}" }
                                        span { class: "task-title", "{task.title}" }
                                    }
                                    div { class: "task-row-bottom",
                                        span { "{task.kind}" }
                                        span { "{priority_str}" }
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
