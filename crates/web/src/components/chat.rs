use crate::mock::{ChatMessage, StatusLine, ToolCall};
use dioxus::document::eval;
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
    let mut input = use_signal(String::new);
    let mut show_model_menu = use_signal(|| false);

    // Auto-scroll chat messages to bottom on message updates
    use_effect(use_reactive(&messages.len(), move |_| {
        eval(
            r#"
            setTimeout(() => {
                const el = document.querySelector(".chat-messages");
                if (el) {
                    el.scrollTop = el.scrollHeight;
                }
            }, 50);
            "#,
        );
    }));

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
                            span { class: "message-avatar", "🤖" }
                            span { class: "message-name", "omenic" }
                            span { class: "message-time", "思考中..." }
                        }
                        div { class: "message-body streaming",
                            span { class: "streaming-dots", "正在连接真实模型并思考生成回答..." }
                        }
                    }
                }
                div { id: "chat-scroll-anchor", class: "chat-bottom-spacer" }
            }

            // Input Docked Floating Box (zcode Style)
            div { class: "chat-input-container",
                div { class: "chat-input-box",
                    // Integrated Status Line at Top of Input
                    div { class: "integrated-statusline",
                        // Model Selector Pill with Dropdown Menu
                        div {
                            class: "statusline-pill model interactive",
                            title: "点击切换模型",
                            onclick: move |_| show_model_menu.set(!show_model_menu()),
                            "🧠 {statusline.model} ▾"
                        }
                        if show_model_menu() {
                            div { class: "model-dropdown-menu",
                                div { class: "model-dropdown-header", "选择推理模型" }
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
                                                    span { "✓" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Thinking Mode Toggle Pill
                        div {
                            class: "statusline-pill thinking interactive",
                            title: "点击循环切换思考强度",
                            onclick: move |_| on_toggle_thinking.call(()),
                            "thinking: {statusline.thinking} 🔄"
                        }

                        // CWD Pill
                        div { class: "statusline-pill cwd",
                            "📁 {statusline.cwd}"
                        }

                        // Git Branch Pill
                        div { class: "statusline-pill branch",
                            "🌿 {statusline.git_branch}"
                        }

                        // Tokens Metrics
                        div { class: "statusline-pill tokens",
                            "📊 {statusline.tokens_in}→{statusline.tokens_out}"
                        }

                        // Cost Pill
                        div { class: "statusline-pill cost",
                            "💰 ${statusline.cost_usd:.3}"
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

                    // Input Text Field
                    textarea {
                        id: "chat-input-area",
                        class: "chat-input-field",
                        placeholder: "输入消息，按 Enter 发送 (Shift+Enter 换行)...",
                        value: "{input}",
                        oninput: move |e| input.set(e.value()),
                    }
                    // Bottom Action Toolbar (zcode Style)
                    div { class: "chat-input-actions",
                        div { class: "input-action-left",
                            button { class: "action-pill-btn", "+ 附件" }
                            button { class: "action-pill-btn", "🛡️ 变更前确认 ▾" }
                        }
                        div { class: "input-action-right",
                            button {
                                class: "btn-send-round",
                                disabled: input().trim().is_empty() || is_streaming,
                                onclick: move |_| {
                                    let content = input().trim().to_string();
                                    if !content.is_empty() && !is_streaming {
                                        input.set(String::new());
                                        on_send.call(content);
                                    }
                                },
                                if is_streaming { "⏳" } else { "↑" }
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

                // Tool calls accordion list (Only rendered when there are real tool calls)
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
        "bash" => "▶",
        "edit" => "✏️",
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
