use crate::mock::{ChatMessage, StatusLine, ToolCall};
use dioxus::prelude::*;
use pulldown_cmark::{html, Options as MarkdownOptions, Parser};

/// 将 Agent/用户消息渲染为 Markdown HTML。
fn markdown_to_html(input: &str) -> String {
    let mut opts = MarkdownOptions::empty();
    opts.insert(MarkdownOptions::ENABLE_STRIKETHROUGH);
    opts.insert(MarkdownOptions::ENABLE_TASKLISTS);
    opts.insert(MarkdownOptions::ENABLE_TABLES);
    let parser = Parser::new_ext(input, opts);
    let mut output = String::new();
    html::push_html(&mut output, parser);
    output
}

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
    let mut show_thinking_menu = use_signal(|| false);

    let models = [
        "agnes-2.5-flash",
        "deepseek-v4-flash",
        "claude-opus-4-7",
        "qwen3-32b",
        "kimi-k3",
    ];

    let thinking_options = [
        ("关闭", "off"),
        ("轻量", "2k"),
        ("标准", "8k"),
        ("深度", "16k"),
    ];

    let display_messages: Vec<_> = messages
        .iter()
        .filter(|m| !m.content.is_empty() || !m.tool_calls.is_empty() || is_streaming)
        .cloned()
        .collect();

    rsx! {
        div { class: "chat-container",
            // Messages stream (centered within max-width: 880px)
            div { class: "chat-messages",
                if display_messages.is_empty() && !is_streaming {
                    div { class: "chat-empty-state",
                        div { class: "chat-empty-title", "开始一个新的任务" }
                        div { class: "chat-empty-hint", "在下方输入指令，Agent 将使用文件读写、bash 与代码编辑工具协助你完成。" }
                    }
                }
                for (idx, msg) in display_messages.iter().enumerate() {
                    if msg.content.is_empty() && msg.tool_calls.is_empty() {
                        div { key: "streaming-{idx}", class: "message assistant",
                            div { class: "message-header",
                                span { class: "message-name", "Agent" }
                                span { class: "message-time", "思考中..." }
                            }
                            div { class: "message-body streaming",
                                span { class: "btn-spinner inline" }
                                span { "正在连接模型并思考生成回答..." }
                            }
                        }
                    } else {
                        MessageBubble { key: "{msg.id}-{idx}", message: msg.clone() }
                    }
                }
                div { id: "chat-scroll-anchor", class: "chat-bottom-spacer" }
            }

            // Input Docked Floating Box (Cursor Composer Aesthetic)
            div { class: "chat-input-container",
                div { class: "chat-input-box",
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
                            placeholder: "输入指令，Enter 发送，Shift+Enter 换行...",
                        }

                        div { class: "chat-input-footer-toolbar",
                            div { class: "footer-left",
                                // Model selector pill
                                div {
                                    class: if show_model_menu() { "toolbar-pill active" } else { "toolbar-pill" },
                                    onclick: move |_| {
                                        show_thinking_menu.set(false);
                                        show_model_menu.set(!show_model_menu());
                                    },
                                    span { "{statusline.model} ▾" }
                                }
                                if show_model_menu() {
                                    div { class: "dropdown-popover",
                                        div { class: "dropdown-header", "选择模型" }
                                        for m in models {
                                            {
                                                let m_str = m.to_string();
                                                let is_active = statusline.model == m;
                                                rsx! {
                                                    div {
                                                        key: "{m}",
                                                        class: if is_active { "dropdown-item active" } else { "dropdown-item" },
                                                        onclick: move |_| {
                                                            on_model_change.call(m_str.clone());
                                                            show_model_menu.set(false);
                                                        },
                                                        span { "{m}" }
                                                        if is_active {
                                                            span { style: "color: var(--accent); font-weight: bold;", "✓" }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                // Thinking selector pill
                                div {
                                    class: if show_thinking_menu() { "toolbar-pill active" } else { "toolbar-pill" },
                                    onclick: move |_| {
                                        show_model_menu.set(false);
                                        show_thinking_menu.set(!show_thinking_menu());
                                    },
                                    span { "思考 {statusline.thinking} ▾" }
                                }
                                if show_thinking_menu() {
                                    div { class: "dropdown-popover",
                                        div { class: "dropdown-header", "思考强度" }
                                        for (label, value) in thinking_options {
                                            {
                                                let is_active = statusline.thinking == value;
                                                rsx! {
                                                    div {
                                                        key: "{value}",
                                                        class: if is_active { "dropdown-item active" } else { "dropdown-item" },
                                                        onclick: move |_| {
                                                            show_thinking_menu.set(false);
                                                            on_toggle_thinking.call(());
                                                        },
                                                        span { "{label}" }
                                                        if is_active {
                                                            span { style: "color: var(--accent); font-weight: bold;", "✓" }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            div { class: "footer-right",
                                span { class: "token-stats", "{statusline.tokens_in} / {statusline.tokens_out}" }
                                button {
                                    r#type: "submit",
                                    class: if is_streaming { "btn-send loading" } else { "btn-send" },
                                    disabled: is_streaming,
                                    if is_streaming {
                                        span { class: "btn-spinner" }
                                        span { "发送中..." }
                                    } else {
                                        span { "发送 ↵" }
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

#[component]
fn MessageBubble(message: ChatMessage) -> Element {
    let is_user = message.role == "user";
    let class = if is_user {
        "message user"
    } else {
        "message assistant"
    };
    let name = if is_user { "You" } else { "Agent" };

    rsx! {
        div { class: "{class}",
            div { class: "message-header",
                span { class: "message-name", "{name}" }
                span { class: "message-time", "{message.timestamp}" }
            }
            div { class: "message-body markdown-body",
                // Rendered markdown
                div { dangerous_inner_html: "{markdown_to_html(&message.content)}" }

                if !message.tool_calls.is_empty() {
                    div { class: "message-tools-container",
                        for (t_idx, tool) in message.tool_calls.iter().enumerate() {
                            ToolAccordion { key: "{tool.id}-{t_idx}", tool: tool.clone() }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ToolAccordion(tool: ToolCall) -> Element {
    let lines_count = tool.detail.lines().count();
    let is_short_content = lines_count <= 6 && tool.detail.len() <= 350;
    let mut is_open = use_signal(move || is_short_content);

    rsx! {
        div { class: "tool-accordion",
            div {
                class: "tool-accordion-header",
                onclick: move |e: MouseEvent| {
                    e.stop_propagation();
                    is_open.set(!is_open());
                },
                div { class: "tool-header-left",
                    span { class: "tool-tag", "{tool.kind}" }
                    span { class: "tool-title-text", "{tool.title}" }
                }
                div { class: "tool-header-right",
                    span { if is_open() { "收起" } else { "详情" } }
                }
            }
            if is_open() {
                div { class: "tool-accordion-content",
                    if !tool.summary.is_empty() {
                        div { class: "tool-summary-text", "{tool.summary}" }
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
