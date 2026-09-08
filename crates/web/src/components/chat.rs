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
        div { class: "flex-1 flex flex-col overflow-hidden",
            // Messages stream
            div { class: "chat-messages flex-1 overflow-y-auto flex flex-col gap-3.5 px-6 pt-5 pb-3 max-w-[1360px] w-[96%] mx-auto",
                if display_messages.is_empty() && !is_streaming {
                    div { class: "flex-1 flex flex-col items-center justify-center gap-2 text-[var(--text-muted)] text-center py-10 select-none",
                        div { class: "text-base font-semibold text-[var(--text-secondary)]", "开始一个新的任务" }
                        div { class: "text-xs max-w-[420px] leading-relaxed", "在下方输入指令，Agent 将使用文件读写、bash 与代码编辑工具协助你完成。" }
                    }
                }
                for (idx, msg) in display_messages.iter().enumerate() {
                    if msg.content.is_empty() && msg.tool_calls.is_empty() {
                        div { key: "streaming-{idx}", class: "self-start max-w-[88%] flex flex-col gap-1",
                            div { class: "flex items-baseline gap-2 text-[11px] mb-0.5",
                                span { class: "font-semibold text-[var(--text-secondary)]", "Agent" }
                                span { class: "text-[10px] text-[var(--text-muted)]", "思考中..." }
                            }
                            div { class: "bg-[#111217] border border-[#1e202c] rounded-[10px] px-3.5 py-2.5 text-[13px] text-[var(--text-muted)] italic",
                                span { class: "btn-spinner inline" }
                                span { "正在连接模型并思考生成回答..." }
                            }
                        }
                    } else {
                        MessageBubble { key: "{msg.id}-{idx}", message: msg.clone() }
                    }
                }
                div { id: "chat-scroll-anchor", class: "h-4 shrink-0" }
            }

            // Input dock
            div { class: "max-w-[1260px] w-[95%] mx-auto px-5 pb-4 relative",
                div { class: "bg-[var(--bg-surface)] border border-[var(--border-subtle)] rounded-xl shadow-[0_8px_26px_rgba(0,0,0,0.42)] flex flex-col transition-all focus-within:border-[var(--accent)]",
                    form {
                        class: "flex flex-col",
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
                            class: "w-full min-h-[68px] max-h-[220px] bg-transparent border-none outline-none text-[var(--text-primary)] font-normal text-[13px] leading-[1.55] px-4 pt-3.5 pb-2 resize-none placeholder:text-[var(--text-muted)]",
                            placeholder: "输入指令，Enter 发送，Shift+Enter 换行...",
                        }

                        div { class: "flex items-center justify-between px-3 pt-2 pb-2.5 border-t border-[rgba(255,255,255,0.04)]",
                            div { class: "flex items-center gap-1",
                                // Model pill
                                div {
                                    class: "relative",
                                    button {
                                        class: "text-[11px] px-2 py-1 rounded text-[var(--text-secondary)] border border-[var(--border-subtle)] bg-transparent hover:border-[var(--border-hover)] transition-colors font-mono",
                                        r#type: "button",
                                        onclick: move |_| {
                                            show_thinking_menu.set(false);
                                            show_model_menu.set(!show_model_menu());
                                        },
                                        "{statusline.model} ▾"
                                    }
                                    if show_model_menu() {
                                        div { class: "absolute bottom-full left-0 mb-1 min-w-[180px] bg-[var(--bg-surface-elevated)] border border-[var(--border-subtle)] rounded-lg shadow-lg z-50 py-1",
                                            div { class: "px-3 py-1.5 text-[10px] font-semibold text-[var(--text-muted)] uppercase tracking-wide", "选择模型" }
                                            for m in models {
                                                {
                                                    let m_str = m.to_string();
                                                    let is_active = statusline.model == m;
                                                    rsx! {
                                                        div {
                                                            key: "{m}",
                                                            class: if is_active { "px-3 py-1.5 text-xs cursor-pointer bg-[var(--bg-hover)] text-[var(--text-primary)] font-medium" } else { "px-3 py-1.5 text-xs cursor-pointer text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]" },
                                                            onclick: move |_| {
                                                                on_model_change.call(m_str.clone());
                                                                show_model_menu.set(false);
                                                            },
                                                            span { class: "font-mono", "{m}" }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                // Thinking pill
                                div {
                                    class: "relative",
                                    button {
                                        class: "text-[11px] px-2 py-1 rounded text-[var(--text-secondary)] border border-[var(--border-subtle)] bg-transparent hover:border-[var(--border-hover)] transition-colors",
                                        r#type: "button",
                                        onclick: move |_| {
                                            show_model_menu.set(false);
                                            show_thinking_menu.set(!show_thinking_menu());
                                        },
                                        "思考 {statusline.thinking} ▾"
                                    }
                                    if show_thinking_menu() {
                                        div { class: "absolute bottom-full left-0 mb-1 min-w-[160px] bg-[var(--bg-surface-elevated)] border border-[var(--border-subtle)] rounded-lg shadow-lg z-50 py-1",
                                            div { class: "px-3 py-1.5 text-[10px] font-semibold text-[var(--text-muted)] uppercase tracking-wide", "思考强度" }
                                            for (label, value) in thinking_options {
                                                {
                                                    let is_active = statusline.thinking == value;
                                                    rsx! {
                                                        div {
                                                            key: "{value}",
                                                            class: if is_active { "px-3 py-1.5 text-xs cursor-pointer bg-[var(--bg-hover)] text-[var(--text-primary)] font-medium" } else { "px-3 py-1.5 text-xs cursor-pointer text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]" },
                                                            onclick: move |_| {
                                                                show_thinking_menu.set(false);
                                                                on_toggle_thinking.call(());
                                                            },
                                                            "{label}"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            div { class: "flex items-center gap-2",
                                span { class: "text-[10px] text-[var(--text-muted)] font-mono", "{statusline.tokens_in} / {statusline.tokens_out}" }
                                button {
                                    r#type: "submit",
                                    class: if is_streaming { "flex items-center gap-1.5 px-3.5 py-1.5 rounded-md text-xs font-semibold bg-[var(--accent)] text-[#0b0c10] opacity-75 cursor-not-allowed" } else { "flex items-center gap-1.5 px-3.5 py-1.5 rounded-md text-xs font-semibold bg-[var(--accent)] text-[#0b0c10] hover:bg-[var(--accent-hover)] cursor-pointer transition-colors" },
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
    let name = if is_user { "You" } else { "Agent" };
    let has_tools = !message.tool_calls.is_empty();
    let mut view_mode = use_signal(|| "result");

    let bubble_class = if is_user {
        "self-end max-w-[82%] flex flex-col gap-1"
    } else {
        "self-start max-w-[920px] w-fit min-w-0 flex flex-col gap-1"
    };

    let body_class = if is_user {
        "bg-[#171520] border border-[#302640] rounded-[10px] px-3 py-2 text-[13px] leading-[1.5] text-[#f1edff] shadow-[0_3px_12px_rgba(0,0,0,0.35)]"
    } else {
        "bg-[#111217] border border-[#1e202c] rounded-[10px] px-3.5 py-2.5 text-[13px] leading-[1.55] text-[var(--text-primary)] shadow-[0_3px_12px_rgba(0,0,0,0.25)]"
    };

    rsx! {
        div { class: "{bubble_class} message",
            div { class: "flex items-baseline gap-2 text-[11px] mb-0.5",
                span { class: "font-semibold text-[var(--text-secondary)]", "{name}" }
                span { class: "text-[10px] text-[var(--text-muted)]", "{message.timestamp}" }
                if has_tools {
                    div { class: "ml-auto inline-flex gap-0.5 bg-[rgba(0,0,0,0.25)] p-0.5 rounded",
                        button {
                            class: if view_mode() == "result" { "px-2 py-0.5 text-[10px] rounded-sm bg-[var(--bg-surface-elevated)] text-[var(--text-primary)] font-semibold" } else { "px-2 py-0.5 text-[10px] rounded-sm text-[var(--text-muted)] hover:text-[var(--text-primary)]" },
                            onclick: move |e| { e.stop_propagation(); view_mode.set("result"); },
                            "结果"
                        }
                        button {
                            class: if view_mode() == "process" { "px-2 py-0.5 text-[10px] rounded-sm bg-[var(--bg-surface-elevated)] text-[var(--text-primary)] font-semibold" } else { "px-2 py-0.5 text-[10px] rounded-sm text-[var(--text-muted)] hover:text-[var(--text-primary)]" },
                            onclick: move |e| { e.stop_propagation(); view_mode.set("process"); },
                            "过程 ({message.tool_calls.len()})"
                        }
                    }
                }
            }
            div { class: "{body_class} message-body",
                if view_mode() == "result" || !has_tools {
                    div { class: "final-content markdown-body", dangerous_inner_html: "{markdown_to_html(&message.content)}" }
                }
                if view_mode() == "process" && has_tools {
                    div { class: "process-panel flex flex-col gap-1 max-h-[420px] overflow-y-auto mt-1 pt-1 border-t border-[rgba(255,255,255,0.05)] w-full box-border",
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
    let mut is_open = use_signal(|| false);

    let (status_color, status_char) = match tool.status.as_str() {
        "running" => ("text-[var(--accent)] animate-spin", "⋯"),
        "error" => ("text-[#f87171]", "✕"),
        _ => ("text-[#34d399]", "✓"),
    };

    let kind_color = if tool.kind == "bash" {
        "text-sky-300 bg-sky-500/10"
    } else if tool.kind == "edit" || tool.kind == "write" {
        "text-emerald-300 bg-emerald-500/10"
    } else if tool.kind == "read" {
        "text-amber-300 bg-amber-500/10"
    } else {
        "text-violet-300 bg-violet-500/10"
    };

    rsx! {
        div { class: "border border-[rgba(255,255,255,0.06)] rounded-md overflow-hidden bg-transparent",
            div {
                class: "flex items-center gap-1.5 px-2 py-1 cursor-pointer select-none text-[11px] leading-snug hover:bg-[rgba(255,255,255,0.04)] transition-colors",
                onclick: move |e: MouseEvent| {
                    e.stop_propagation();
                    is_open.set(!is_open());
                },
                span { class: "{status_color} font-mono text-[10px] w-3.5 text-center shrink-0", "{status_char}" }
                span { class: "{kind_color} font-mono text-[9px] font-bold uppercase px-1.5 py-px rounded-sm shrink-0", "{tool.kind}" }
                span { class: "font-mono text-[11px] text-[var(--text-primary)] flex-1 truncate min-w-0", "{tool.title}" }
                span { class: "text-[9px] text-[var(--text-muted)] shrink-0", if is_open() { "▾" } else { "▸" } }
            }
            if is_open() {
                div { class: "px-2.5 py-2 border-t border-[rgba(255,255,255,0.05)] bg-[#09090c]",
                    if !tool.summary.is_empty() {
                        div { class: "text-[11px] text-[var(--text-secondary)] mb-1.5 leading-relaxed", "{tool.summary}" }
                    }
                    div { class: "bg-[#050507] border border-[rgba(255,255,255,0.05)] rounded px-2.5 py-2 max-h-[260px] overflow-y-auto",
                        pre { class: "font-mono text-[11px] leading-[1.5] text-[#c9c9d1] whitespace-pre-wrap break-all m-0",
                            for line in tool.detail.lines() {
                                if line.starts_with('+') && !line.starts_with("+++") {
                                    span { class: "text-[#86efac]", "{line}\n" }
                                } else if line.starts_with('-') && !line.starts_with("---") {
                                    span { class: "text-[#f87171]", "{line}\n" }
                                } else if line.starts_with("@@") {
                                    span { class: "text-[var(--accent)]", "{line}\n" }
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
