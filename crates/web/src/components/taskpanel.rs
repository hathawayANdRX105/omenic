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
        div {
            class: "floating-task-window",
            div {
                class: "floating-task-window-header",
                div { class: "floating-task-window-title", "TASKS" }
                button {
                    class: "floating-task-window-close",
                    onclick: move |_| on_close.call(()),
                    "[收起 ✕]"
                }
            }
            div {
                class: "floating-task-window-content",
                div {
                    class: "task-filter-group",
                    for filter in ["all", "open", "in_progress", "done"] {
                        button {
                            class: if selected_filter() == filter {
                                "task-filter-pill active"
                            } else {
                                "task-filter-pill"
                            },
                            onclick: move |_| selected_filter.set(filter.to_string()),
                            match filter {
                                "all" => "全部",
                                "open" => "进行中",
                                "in_progress" => "进行中",
                                "done" => "已完成",
                                _ => filter,
                            }
                        }
                    }
                }
                div {
                    class: "progress-task-list",
                    for task in filtered_tasks {
                        {
                            let is_selected = selected_task_id() == Some(task.id.clone());
                            let dot_class = if task.status == "open" {
                                "open"
                            } else if task.status == "in_progress" {
                                "in_progress"
                            } else if task.status == "blocked" {
                                "blocked"
                            } else if task.status == "done" {
                                "done"
                            } else {
                                ""
                            };
                            let priority_class = match task.priority {
                                0 => "p0",
                                1 => "p1",
                                2 => "p2",
                                _ => "",
                            };
                            let task_id = task.id.clone();

                            rsx! {
                                div {
                                    class: if is_selected {
                                        "progress-task-item selected"
                                    } else {
                                        "progress-task-item"
                                    },
                                    onclick: move |_| selected_task_id.set(Some(task_id.clone())),
                                    div {
                                        class: "task-row-top",
                                        div { class: "task-status-dot {dot_class}" }
                                        div { class: "task-title-text", "{task.title}" }
                                        div { class: "task-priority-pill {priority_class}", "{task.priority}" }
                                    }
                                    if !task.description.is_empty() {
                                        div { class: "task-detail-drawer", "{task.description}" }
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
