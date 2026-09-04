use crate::mock::TaskItem;
use dioxus::prelude::*;

#[component]
pub fn TaskPanel(tasks: Vec<TaskItem>) -> Element {
    let mut expanded = use_signal(|| false);
    let mut selected_filter = use_signal(|| "all".to_string());
    let mut selected_task_id = use_signal(|| None::<String>);

    let status_icon = |s: &str| match s {
        "open" => "🔲",
        "in_progress" => "🔄",
        "blocked" => "🚫",
        "done" => "✅",
        _ => "❓",
    };

    let open_count = tasks.iter().filter(|t| t.status == "open").count();
    let in_progress = tasks.iter().filter(|t| t.status == "in_progress").count();
    let blocked = tasks.iter().filter(|t| t.status == "blocked").count();
    let done = tasks.iter().filter(|t| t.status == "done").count();
    let total = tasks.len();

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
        div { class: "task-panel",
            button {
                class: "task-toggle",
                onclick: move |_| expanded.set(!expanded()),
                if expanded() {
                    span { "▾ 任务看板 ({open_count + in_progress + blocked} 待办)" }
                } else {
                    span { "▸ 任务看板 ({open_count + in_progress + blocked})" }
                }
            }
            if expanded() {
                div { class: "task-panel-body",
                    // Filter tabs
                    div { class: "task-summary",
                        button {
                            class: if selected_filter() == "all" { "task-filter-btn active" } else { "task-filter-btn" },
                            onclick: move |_| selected_filter.set("all".into()),
                            "全部 ({total})"
                        }
                        button {
                            class: if selected_filter() == "in_progress" { "task-filter-btn active" } else { "task-filter-btn" },
                            onclick: move |_| selected_filter.set("in_progress".into()),
                            "🔄 进行中 ({in_progress})"
                        }
                        button {
                            class: if selected_filter() == "open" { "task-filter-btn active" } else { "task-filter-btn" },
                            onclick: move |_| selected_filter.set("open".into()),
                            "🔲 待办 ({open_count})"
                        }
                        button {
                            class: if selected_filter() == "blocked" { "task-filter-btn active" } else { "task-filter-btn" },
                            onclick: move |_| selected_filter.set("blocked".into()),
                            "🚫 阻塞 ({blocked})"
                        }
                        button {
                            class: if selected_filter() == "done" { "task-filter-btn active" } else { "task-filter-btn" },
                            onclick: move |_| selected_filter.set("done".into()),
                            "✅ 完成 ({done})"
                        }
                    }

                    // Task List
                    div { class: "task-list",
                        for task in filtered_tasks {
                            {
                                let is_selected = selected_task_id().as_deref() == Some(&task.id);
                                let task_id = task.id.clone();
                                rsx! {
                                    div {
                                        key: "{task.id}",
                                        class: if is_selected { "task-item task-{task.status} selected" } else { "task-item task-{task.status}" },
                                        onclick: move |_| {
                                            if is_selected {
                                                selected_task_id.set(None);
                                            } else {
                                                selected_task_id.set(Some(task_id.clone()));
                                            }
                                        },
                                        div { class: "task-item-row",
                                            span { class: "task-status-icon", "{status_icon(&task.status)}" }
                                            div { class: "task-info",
                                                div { class: "task-title", "{task.title}" }
                                                div { class: "task-meta",
                                                    span { class: "task-kind", "{task.kind}" }
                                                    span { class: "task-priority", "P{task.priority}" }
                                                    span { class: "task-id-badge", "#{task.id}" }
                                                }
                                            }
                                            span { class: "task-expand-arrow", if is_selected { "▾" } else { "▸" } }
                                        }

                                        // Task details card when clicked
                                        if is_selected {
                                            div { class: "task-detail-card",
                                                if !task.description.is_empty() {
                                                    div { class: "task-desc-row",
                                                        span { class: "task-desc-label", "说明：" }
                                                        span { "{task.description}" }
                                                    }
                                                }
                                                if !task.acceptance.is_empty() {
                                                    div { class: "task-desc-row",
                                                        span { class: "task-desc-label", "验收：" }
                                                        span { "{task.acceptance}" }
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
}
