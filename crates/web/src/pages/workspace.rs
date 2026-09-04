use crate::components::chat::Chat;
use crate::components::sidebar::Sidebar;
use crate::components::taskpanel::TaskPanel;
use crate::mock;
use dioxus::prelude::*;

#[component]
pub fn Workspace() -> Element {
    let sessions = mock::mock_sessions();
    let mut active_session = use_signal(|| "s1".to_string());
    let mut messages = use_signal(mock::mock_messages);
    let statusline = mock::mock_statusline();
    let tasks = mock::mock_tasks();

    let on_send = move |text: String| {
        let new_msg = mock::ChatMessage {
            id: format!("m{}", messages().len() + 1),
            role: "user".into(),
            content: text.clone(),
            timestamp: "刚刚".into(),
        };
        messages.push(new_msg);

        // Mock echo response
        let echo = mock::ChatMessage {
            id: format!("m{}", messages().len() + 1),
            role: "assistant".into(),
            content: format!("收到：{}\n\n（mock 模式：尚未接真实 LLM）", text),
            timestamp: "刚刚".into(),
        };
        messages.push(echo);
    };

    rsx! {
        div { class: "workspace-layout",
            Sidebar {
                sessions: sessions,
                active_id: active_session(),
                on_select: move |id| active_session.set(id),
            }
            div { class: "workspace-main",
                div { class: "workspace-chat-area",
                    Chat {
                        messages: messages(),
                        statusline: statusline,
                        on_send: on_send,
                    }
                }
                div { class: "workspace-side",
                    TaskPanel { tasks: tasks }
                }
            }
        }
    }
}
