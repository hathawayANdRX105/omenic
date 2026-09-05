use crate::mock::TaskItem;
use dioxus::prelude::*;

#[component]
pub fn TaskPanel(tasks: Vec<TaskItem>, on_close: EventHandler<()>) -> Element {
    let mut selected_filter = use_signal(|| "all".to_string());
    let mut selected_task_id = use_signal(|| None::<String>);

    let open_count = tasks.iter().filter(|t| t.status == "open").count();
    let in_progress = tasks.iter().filter(|t| t.status == "in_progress").count();
    let _blocked = tasks.iter().filter(|t| t.status == "blocked").count();
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
        div { class: "workspace-side-panel",
            // Header
            div { class: "side-panel-header",
                div { class: "side-panel-title", "编排任务与进度" }
                button {
                    class: "btn-toggle-taskpanel",
                    onclick: move |_| on_close.call(()),
                    "收起"
                }
            }

            div { class: "side-panel-content",
                // Git tools card (zcode style)
                div { class: "goal-card",
                    div { class: "goal-card-header",
                        span { class: "goal-label", "Git 状态" }
                        span { class: "goal-badge", "feat/web-dioxus" }
                    }
                    div { style: "display: flex; justify-content: space-between; font-size: 11.5px; color: var(--text-secondary);",
                        span { "变更: +1271 -977" }
                        span { style: "color: var(--accent-green);", "已提交" }
                    }
                }

                // Goal card (zcode style)
                div { class: "goal-card",
                    div { class: "goal-card-header",
                        span { class: "goal-label", "总体目标" }
                        span {
                            class: "goal-badge",
                            if done == total { "Complete" } else { "In Progress" }
                        }
                    }
                    div { class: "goal-title", "任务编排看板 (DAG 链路)" }
                    div { class: "goal-stats", "{done}/{total} 已完成 · 89K tokens" }
                }

                // Progress checklist section (zcode style)
                div {
                    div { class: "progress-section-header",
                        span { class: "progress-title", "步骤清单" }
                        div { class: "task-filter-group",
                            button {
                                class: if selected_filter() == "all" { "task-filter-pill active" } else { "task-filter-pill" },
                                onclick: move |_| selected_filter.set("all".into()),
                                "全部"
                            }
                            button {
                                class: if selected_filter() == "in_progress" { "task-filter-pill active" } else { "task-filter-pill" },
                                onclick: move |_| selected_filter.set("in_progress".into()),
                                "进行中 ({in_progress})"
                            }
                            button {
                                class: if selected_filter() == "open" { "task-filter-pill active" } else { "task-filter-pill" },
                                onclick: move |_| selected_filter.set("open".into()),
                                "待办 ({open_count})"
                            }
                            button {
                                class: if selected_filter() == "done" { "task-filter-pill active" } else { "task-filter-pill" },
                                onclick: move |_| selected_filter.set("done".into()),
                                "完成 ({done})"
                            }
                        }
                    }

                    div { class: "progress-task-list",
                        for task in filtered_tasks {
                            {
                                let is_selected = selected_task_id().as_deref() == Some(&task.id);
                                let task_id = task.id.clone();
                                let p_class = format!("p{}", task.priority);
                                let dot_status = task.status.clone();
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
                                            span { class: "task-status-dot {dot_status}" }
                                            span { class: "task-title-text", "{task.title}" }
                                            span { class: "task-priority-pill {p_class}", "P{task.priority}" }
                                        }
                                        if is_selected {
                                            div { class: "task-detail-drawer",
                                                if !task.description.is_empty() {
                                                    div { "说明: {task.description}" }
                                                }
                                                if !task.acceptance.is_empty() {
                                                    div { style: "color: var(--accent-blue);", "验收: {task.acceptance}" }
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
