use crate::mock::{ChatMessage, StatusLine};
use dioxus::prelude::*;

#[component]
pub fn Chat(
    messages: Vec<ChatMessage>,
    statusline: StatusLine,
    on_send: EventHandler<String>,
) -> Element {
    let mut input = use_signal(String::new);

    rsx! {
        div { class: "chat-container",
            // Messages area
            div { class: "chat-messages",
                for msg in &messages {
                    MessageBubble { key: "{msg.id}", message: msg.clone() }
                }
            }

            // Status line (above input)
            div { class: "statusline",
                span { class: "statusline-model", "🧠 {statusline.model}" }
                span { class: "statusline-sep", "│" }
                span { class: "statusline-thinking", "thinking: {statusline.thinking}" }
                span { class: "statusline-sep", "│" }
                span { class: "statusline-cwd", "📁 {statusline.cwd}" }
                span { class: "statusline-sep", "│" }
                span { class: "statusline-branch", "🌿 {statusline.git_branch}" }
                span { class: "statusline-sep", "│" }
                span { class: "statusline-tokens", "📊 {statusline.tokens_in}→{statusline.tokens_out}" }
                span { class: "statusline-sep", "│" }
                span { class: "statusline-cost", "💰 ${statusline.cost_usd:.3}" }
                span { class: "statusline-sep", "│" }
                div { class: "statusline-context",
                    div { class: "context-bar-bg",
                        div {
                            class: "context-bar-fill",
                            style: "width: {statusline.context_pct}%"
                        }
                    }
                    span { "{statusline.context_pct}%" }
                }
            }

            // Input area
            div { class: "chat-input-area",
                div { class: "chat-input-row",
                    input {
                        class: "chat-input",
                        placeholder: "输入消息...",
                        value: "{input}",
                        oninput: move |e| input.set(e.value()),
                        onkeydown: move |e| {
                            if e.key() == Key::Enter && !input().is_empty() {
                                on_send.call(input());
                                input.set(String::new());
                            }
                        },
                    }
                    button {
                        class: "btn-send",
                        onclick: move |_| {
                            if !input().is_empty() {
                                on_send.call(input());
                                input.set(String::new());
                            }
                        },
                        "发送"
                    }
                }
                div { class: "chat-options",
                    ModelSelector {}
                }
            }
        }
    }
}

#[component]
fn MessageBubble(message: ChatMessage) -> Element {
    let is_user = message.role == "user";
    let class = if is_user {
        "message user"
    } else {
        "message assistant"
    };
    let (avatar, name) = if is_user {
        ("👤", "你")
    } else {
        ("🤖", "omenic")
    };

    rsx! {
        div { class: "{class}",
            div { class: "message-header",
                span { class: "message-avatar", "{avatar}" }
                span { class: "message-name", "{name}" }
                span { class: "message-time", "{message.timestamp}" }
            }
            div { class: "message-body",
                for line in message.content.lines() {
                    if line.starts_with("已运行") || line.starts_with("已写入") {
                        div { class: "message-tool-line", "{line}" }
                    } else if line.starts_with("- ") {
                        div { class: "message-bullet", "{line}" }
                    } else if line.is_empty() {
                        br {}
                    } else {
                        p { "{line}" }
                    }
                }
            }
        }
    }
}

#[component]
fn ModelSelector() -> Element {
    let mut open = use_signal(|| false);
    let mut current = use_signal(|| "qwen3-32b".to_string());
    let models = ["qwen3-32b", "agnes-2.5-flash", "kimi-k3", "deepseek-v3"];

    rsx! {
        div { class: "model-selector",
            button {
                class: "btn-model",
                onclick: move |_| open.set(!open()),
                "模型: {current}"
                span { class: "caret", " ▾" }
            }
            if open() {
                div { class: "model-dropdown",
                    for m in models {
                        div {
                            key: "{m}",
                            class: if *current.read() == m { "model-option selected" } else { "model-option" },
                            onclick: move |_| {
                                current.set(m.to_string());
                                open.set(false);
                            },
                            "{m}"
                        }
                    }
                }
            }
        }
    }
}
