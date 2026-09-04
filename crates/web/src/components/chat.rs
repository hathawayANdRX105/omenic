use crate::mock::{ChatMessage, StatusLine, ToolCall};
use dioxus::prelude::*;

#[component]
pub fn Chat(
    messages: Vec<ChatMessage>,
    statusline: StatusLine,
    on_send: EventHandler<String>,
    on_model_change: EventHandler<String>,
    on_toggle_thinking: EventHandler<()>,
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
                span {
                    class: "statusline-model clickable",
                    title: "当前使用的模型",
                    "🧠 {statusline.model}"
                }
                span { class: "statusline-sep", "│" }
                span {
                    class: "statusline-thinking clickable",
                    title: "点击切换 thinking 模式",
                    onclick: move |_| on_toggle_thinking.call(()),
                    "thinking: {statusline.thinking} 🔄"
                }
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
                        placeholder: "输入消息（按 Enter 发送）...",
                        value: "{input}",
                        oninput: move |e| input.set(e.value()),
                        onkeydown: move |e| {
                            if e.key() == Key::Enter && !input().trim().is_empty() {
                                on_send.call(input().trim().to_string());
                                input.set(String::new());
                            }
                        },
                    }
                    button {
                        class: "btn-send",
                        disabled: input().trim().is_empty(),
                        onclick: move |_| {
                            if !input().trim().is_empty() {
                                on_send.call(input().trim().to_string());
                                input.set(String::new());
                            }
                        },
                        "发送"
                    }
                }
                div { class: "chat-options",
                    ModelSelector {
                        current: statusline.model.clone(),
                        on_select: on_model_change,
                    }
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
                    if line.starts_with("- ") {
                        div { class: "message-bullet", "{line}" }
                    } else if line.is_empty() {
                        br {}
                    } else {
                        p { "{line}" }
                    }
                }

                // Tool calls accordion list
                if !message.tool_calls.is_empty() {
                    div { class: "message-tools-container",
                        div { class: "tools-header-label", "调用工具与执行结果（点击展开详情）：" }
                        for tool in &message.tool_calls {
                            ToolAccordion { key: "{tool.id}", tool: tool.clone() }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ToolAccordion(tool: ToolCall) -> Element {
    let mut is_open = use_signal(|| false);

    let icon = match tool.kind.as_str() {
        "bash" => "⚙️",
        "edit" => "📝",
        "read" => "🔍",
        _ => "⚡",
    };

    rsx! {
        div { class: if is_open() { "tool-accordion open" } else { "tool-accordion" },
            div {
                class: "tool-accordion-header",
                onclick: move |_| is_open.set(!is_open()),
                div { class: "tool-header-left",
                    span { class: "tool-toggle-icon", if is_open() { "▾" } else { "▸" } }
                    span { class: "tool-kind-icon", "{icon}" }
                    span { class: "tool-title-text", "{tool.title}" }
                }
                div { class: "tool-header-right",
                    span { class: "tool-status-badge {tool.status}", "✓ 成功" }
                    span { class: "tool-expand-hint", if is_open() { "收起" } else { "展开详情" } }
                }
            }
            if is_open() {
                div { class: "tool-accordion-content",
                    if !tool.summary.is_empty() {
                        div { class: "tool-summary-line", "概要: {tool.summary}" }
                    }
                    div { class: "tool-detail-box",
                        pre { class: "tool-detail-code",
                            for line in tool.detail.lines() {
                                if line.starts_with('+') && !line.starts_with("+++") {
                                    span { class: "diff-add", "{line}\n" }
                                } else if line.starts_with('-') && !line.starts_with("---") {
                                    span { class: "diff-del", "{line}\n" }
                                } else if line.starts_with("@@") {
                                    span { class: "diff-hunk", "{line}\n" }
                                } else {
                                    span { "{line}\n" }
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
fn ModelSelector(current: String, on_select: EventHandler<String>) -> Element {
    let mut open = use_signal(|| false);
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
                        {
                            let m_str = m.to_string();
                            rsx! {
                                div {
                                    key: "{m}",
                                    class: if current == m { "model-option selected" } else { "model-option" },
                                    onclick: move |_| {
                                        on_select.call(m_str.clone());
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
    }
}
