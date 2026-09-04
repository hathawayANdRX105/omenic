use crate::mock::TaskItem;
use dioxus::prelude::*;

#[component]
pub fn TaskPanel(tasks: Vec<TaskItem>) -> Element {
    let mut expanded = use_signal(|| false);

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

    rsx! {
        div { class: "task-panel",
            button {
                class: "task-toggle",
                onclick: move |_| expanded.set(!expanded()),
                if expanded() { "▸ 任务看板" } else { "▸ 任务看板 ({open_count + in_progress + blocked})" }
            }
            if expanded() {
                div { class: "task-panel-body",
                    div { class: "task-summary",
                        span { class: "task-count open", "🔲 {open_count}" }
                        span { class: "task-count in-progress", "🔄 {in_progress}" }
                        span { class: "task-count blocked", "🚫 {blocked}" }
                        span { class: "task-count done", "✅ {done}" }
                    }
                    div { class: "task-list",
                        for task in &tasks {
                            div {
                                key: "{task.id}",
                                class: "task-item task-{task.status}",
                                span { class: "task-status-icon", "{status_icon(&task.status)}" }
                                div { class: "task-info",
                                    div { class: "task-title", "{task.title}" }
                                    div { class: "task-meta",
                                        span { class: "task-kind", "{task.kind}" }
                                        span { class: "task-priority", "P{task.priority}" }
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
