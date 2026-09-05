use crate::mock::{ChatMessage, StatusLine, ToolCall};
use dioxus::prelude::*;

#[component]
pub fn Chat(
    messages: Vec<ChatMessage>,
    statusline: StatusLine,
    is_streaming: bool,
    on_send: EventHandler<String>,
    on_model_change: EventHandler<String>,
    on_toggle_thinking: EventHandler<()>,
) -> Element {
    let mut show_model_menu = use_signal(|| false);

    let models = [
        "agnes-2.5-flash",
        "deepseek-v4-flash",
        "claude-opus-4-7",
        "qwen3-32b",
        "kimi-k3",
    ];

    rsx! {
        div { class: "chat-container",
            // Messages area
            div { class: "chat-messages",
                for msg in &messages {
                    MessageBubble { key: "{msg.id}", message: msg.clone() }
                }
                if is_streaming {
                    div { class: "message assistant",
                        div { class: "message-header",
                            span { class: "message-name", "omenic" }
                            span { class: "message-time", "思考中..." }
                        }
                        div { class: "message-body streaming",
                            "正在连接模型并思考生成回答..."
                        }
                    }
                }
                div { id: "chat-scroll-anchor", class: "chat-bottom-spacer" }
            }

            // Input Docked Floating Box (zcode / Linear Style)
            div { class: "chat-input-container",
                div { class: "chat-input-box",
                    // Integrated Status Line (zcode Style, Clean & Text-focused)
                    div { class: "integrated-statusline",
                        // Model Selector Pill
                        div {
                            class: "statusline-pill model interactive",
                            title: "点击切换当前模型",
                            onclick: move |_| show_model_menu.set(!show_model_menu()),
                            span { "model: {statusline.model} ▾" }
                        }
                        if show_model_menu() {
                            div { class: "model-dropdown-menu",
                                div { class: "model-dropdown-header", "选择模型" }
                                for m in models {
                                    {
                                        let m_str = m.to_string();
                                        let is_active = statusline.model == m;
                                        rsx! {
                                            div {
                                                key: "{m}",
                                                class: if is_active { "model-dropdown-item active" } else { "model-dropdown-item" },
                                                onclick: move |_| {
                                                    on_model_change.call(m_str.clone());
                                                    show_model_menu.set(false);
                                                },
                                                span { "{m}" }
                                                if is_active {
                                                    span { style: "font-size: 11px;", "✓" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Thinking Mode Pill
                        div {
                            class: "statusline-pill thinking interactive",
                            title: "点击切换思考强度",
                            onclick: move |_| on_toggle_thinking.call(()),
                            "thinking: {statusline.thinking}"
                        }

                        // CWD Pill
                        div { class: "statusline-pill cwd",
                            "{statusline.cwd}"
                        }

                        // Branch Pill
                        div { class: "statusline-pill branch",
                            "{statusline.git_branch}"
                        }

                        // Tokens Metric Pill
                        div { class: "statusline-pill tokens",
                            "{statusline.tokens_in} / {statusline.tokens_out}"
                        }

                        // Cost Pill
                        div { class: "statusline-pill cost",
                            "${statusline.cost_usd:.3}"
                        }

                        // Context Bar Pill
                        div { class: "statusline-pill context",
                            div { class: "context-bar-bg",
                                div {
                                    class: "context-bar-fill",
                                    style: "width: {statusline.context_pct}%",
                                }
                            }
                            span { "{statusline.context_pct}%" }
                        }
                    }

                    // Form Container with Native Form Submit
                    form {
                        class: "chat-input-form",
                        onsubmit: move |e: FormEvent| {
                            let values = e.values();
                            let text = values
                                .get("message")
                                .and_then(|v| v.first())
                                .map(|s| s.trim().to_string())
                                .unwrap_or_default();
                            if !text.is_empty() && !is_streaming {
                                on_send.call(text);
                            }
                        },

                        textarea {
                            id: "chat-input-area",
                            name: "message",
                            class: "chat-input-field",
                            placeholder: "输入消息，Enter 发送，Shift+Enter 换行...",
                        }

                        div { class: "chat-input-actions",
                            div { class: "input-action-left",
                                button {
                                    r#type: "button",
                                    class: "action-pill-btn",
                                    "确认模式: 自动"
                                }
                            }
                            div { class: "input-action-right",
                                button {
                                    r#type: "submit",
                                    class: "btn-send-round",
                                    if is_streaming { "Sending..." } else { "Send" }
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
fn MessageBubble(message: ChatMessage) -> Element {
    let is_user = message.role == "user";
    let class = if is_user {
        "message user"
    } else {
        "message assistant"
    };
    let name = if is_user { "You" } else { "omenic" };

    rsx! {
        div { class: "{class}",
            div { class: "message-header",
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

                // Tool calls list (only when tools were genuinely called)
                if !message.tool_calls.is_empty() {
                    div { class: "message-tools-container",
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

    rsx! {
        div { class: "tool-accordion",
            div {
                class: "tool-accordion-header",
                onclick: move |_| is_open.set(!is_open()),
                div { class: "tool-header-left",
                    span { class: "tool-tag", "{tool.kind}" }
                    span { class: "tool-title-text", "{tool.title}" }
                }
                div { class: "tool-header-right",
                    span { if is_open() { "hide" } else { "details" } }
                }
            }
            if is_open() {
                div { class: "tool-accordion-content",
                    if !tool.summary.is_empty() {
                        div { style: "font-size: 12px; color: var(--text-secondary); margin-bottom: 6px;", "{tool.summary}" }
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
