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

    let last_idx = display_messages.len().saturating_sub(1);

    rsx! {
        div { class: "relative flex-1 min-h-0 overflow-hidden",
            // 单一滚动面板 = 整个聊天室（只有一个滚动条，位于房间最右侧）
            div { class: "absolute inset-0 overflow-y-auto",
                div { class: "max-w-[1360px] w-[96%] mx-auto px-6 pt-5 pb-[160px] flex flex-col gap-3.5 min-h-full",
                if display_messages.is_empty() && !is_streaming {
                    div { class: "flex-1 flex flex-col items-center justify-center gap-2 text-muted text-center py-10 select-none",
                        div { class: "text-base font-semibold text-secondary", "开始一个新的任务" }
                        div { class: "text-xs max-w-[420px] leading-relaxed", "在下方输入指令，Agent 将使用文件读写、bash 与代码编辑工具协助你完成。" }
                    }
                }
                for (idx, msg) in display_messages.iter().enumerate() {
                    if msg.content.is_empty() && msg.tool_calls.is_empty() {
                        div { key: "streaming-{idx}", class: "self-start max-w-[88%] flex flex-col gap-1",
                            div { class: "bg-surface border border-subtle rounded-[10px] px-3.5 py-2.5 text-[13px] text-muted italic",
                                span { class: "inline-block w-3 h-3 border-2 border-[rgba(255,255,255,0.25)] border-t-accent rounded-full animate-spin" }
                                span { "正在连接模型并思考生成回答..." }
                            }
                        }
                    } else {
                        MessageBubble { key: "{msg.id}-{idx}", message: msg.clone() }
                    }
                    if msg.role == "agent" && idx != last_idx {
                        div { class: "h-px w-full bg-subtle/40 my-2" }
                    }
                }
                div { id: "chat-scroll-anchor", class: "h-4 shrink-0" }
                }
            }

            // 输入框悬浮在聊天室上方（pointer-events-none 让滚动穿透，仅输入框本身可交互）
            div { class: "absolute bottom-0 left-1/2 -translate-x-1/2 w-[95%] max-w-[1260px] px-5 pb-4 z-10 pointer-events-none",
                div { class: "bg-surface border border-subtle rounded-xl shadow-[0_8px_26px_rgba(0,0,0,0.42)] flex flex-col transition-all focus-within:border-accent pointer-events-auto",
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
                            class: "w-full min-h-[68px] max-h-[220px] bg-transparent border-none outline-none text-primary font-normal text-[13px] leading-[1.55] px-4 pt-3.5 pb-2 resize-none placeholder:text-muted",
                            placeholder: "输入指令，Enter 发送，Shift+Enter 换行...",
                        }

                        div { class: "flex items-center justify-between px-3 pt-2 pb-2.5 border-t border-[rgba(255,255,255,0.04)]",
                            div { class: "flex items-center gap-1",
                                // Model pill
                                div {
                                    class: "relative",
                                    button {
                                        class: "text-[11px] px-2 py-1 rounded text-secondary border border-subtle bg-transparent hover:border-hover transition-colors font-mono",
                                        r#type: "button",
                                        onclick: move |_| {
                                            show_thinking_menu.set(false);
                                            show_model_menu.set(!show_model_menu());
                                        },
                                        "{statusline.model} ▾"
                                    }
                                    if show_model_menu() {
                                        div { class: "absolute bottom-full left-0 mb-1 min-w-[180px] bg-surface-elevated border border-subtle rounded-lg shadow-lg z-50 py-1",
                                            div { class: "px-3 py-1.5 text-[10px] font-semibold text-muted uppercase tracking-wide", "选择模型" }
                                            for m in models {
                                                {
                                                    let m_str = m.to_string();
                                                    let is_active = statusline.model == m;
                                                    rsx! {
                                                        div {
                                                            key: "{m}",
                                                            class: if is_active { "px-3 py-1.5 text-xs cursor-pointer bg-hover text-primary font-medium" } else { "px-3 py-1.5 text-xs cursor-pointer text-secondary hover:bg-hover hover:text-primary" },
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
                                        class: "text-[11px] px-2 py-1 rounded text-secondary border border-subtle bg-transparent hover:border-hover transition-colors",
                                        r#type: "button",
                                        onclick: move |_| {
                                            show_model_menu.set(false);
                                            show_thinking_menu.set(!show_thinking_menu());
                                        },
                                        "思考 {statusline.thinking} ▾"
                                    }
                                    if show_thinking_menu() {
                                        div { class: "absolute bottom-full left-0 mb-1 min-w-[160px] bg-surface-elevated border border-subtle rounded-lg shadow-lg z-50 py-1",
                                            div { class: "px-3 py-1.5 text-[10px] font-semibold text-muted uppercase tracking-wide", "思考强度" }
                                            for (label, value) in thinking_options {
                                                {
                                                    let is_active = statusline.thinking == value;
                                                    rsx! {
                                                        div {
                                                            key: "{value}",
                                                            class: if is_active { "px-3 py-1.5 text-xs cursor-pointer bg-hover text-primary font-medium" } else { "px-3 py-1.5 text-xs cursor-pointer text-secondary hover:bg-hover hover:text-primary" },
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
                                span { class: "text-[10px] text-muted font-mono", "{statusline.tokens_in} / {statusline.tokens_out}" }
                                button {
                                    r#type: "submit",
                                    class: if is_streaming { "flex items-center gap-1.5 px-3.5 py-1.5 rounded-md text-xs font-semibold bg-accent text-[#0b0c10] opacity-75 cursor-not-allowed" } else { "flex items-center gap-1.5 px-3.5 py-1.5 rounded-md text-xs font-semibold bg-accent text-[#0b0c10] hover:bg-accent-hover cursor-pointer transition-colors" },
                                    disabled: is_streaming,
                                    if is_streaming {
                                        span { class: "inline-block w-3 h-3 border-2 border-[rgba(255,255,255,0.25)] border-t-accent rounded-full animate-spin" }
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

    if is_user {
        // 用户消息：右侧消息气泡（无表头，模拟 zcode 风格）
        rsx! {
            div { class: "flex flex-col items-end gap-1 w-full",
                if !message.content.is_empty() {
                    div { class: "markdown-body text-primary bg-surface-elevated border border-accent-subtle rounded-[10px] px-3 py-2 text-[13px] leading-[1.55] max-w-[82%]",
                        dangerous_inner_html: "{markdown_to_html(&message.content)}"
                    }
                }
            }
        }
    } else {
        // 助手消息：先「工作过程」(可折叠)，再最终结果（始终显示）
        let has_tools = !message.tool_calls.is_empty();
        rsx! {
            div { class: "flex flex-col gap-3 w-full",
                if has_tools {
                    WorkProcessCollapsible { tools: message.tool_calls.clone() }
                }
                if !message.content.is_empty() {
                    div { class: "markdown-body text-primary",
                        dangerous_inner_html: "{markdown_to_html(&message.content)}"
                    }
                }
            }
        }
    }
}

/// 把工具调用按类型(kind)分组，保持类型首次出现顺序
fn group_tools(tools: &[ToolCall]) -> Vec<(String, Vec<ToolCall>)> {
    let mut groups: Vec<(String, Vec<ToolCall>)> = Vec::new();
    for tc in tools {
        if let Some(slot) = groups.iter_mut().find(|g| g.0 == tc.kind) {
            slot.1.push(tc.clone());
        } else {
            groups.push((tc.kind.clone(), vec![tc.clone()]));
        }
    }
    groups
}

#[component]
fn WorkProcessCollapsible(tools: Vec<ToolCall>) -> Element {
    let mut is_open = use_signal(|| false);
    let groups = group_tools(&tools);

    rsx! {
        div { class: "flex flex-col",
            div {
                class: "flex items-center gap-1.5 cursor-pointer select-none text-[11px] text-muted hover:text-secondary transition-colors py-0.5",
                onclick: move |e: MouseEvent| { e.stop_propagation(); is_open.set(!is_open()); },
                span { class: "font-mono w-3.5 text-center shrink-0", if is_open() { "▾" } else { "▸" } }
                span { class: "font-medium text-secondary", "工作过程" }
                span { class: "text-muted", " · {tools.len()}" }
            }
            if is_open() {
                div { class: "flex flex-col gap-2 pl-3 mt-1",
                    for (kind, group) in groups.iter() {
                        div { class: "flex flex-col gap-0.5",
                            div { class: "text-[10px] font-mono text-muted uppercase tracking-wide", "{kind} · {group.len()}" }
                            for tool in group.iter() {
                                ProcessToolRow { key: "{tool.id}", tool: tool.clone() }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ProcessToolRow(tool: ToolCall) -> Element {
    let mut is_open = use_signal(|| false);

    let (status_color, status_char) = match tool.status.as_str() {
        "running" => ("text-accent animate-spin", "⋯"),
        "error" => ("text-danger", "✕"),
        _ => ("text-success", "✓"),
    };

    let kind_color = if tool.kind == "bash" {
        "text-sky-300/90 bg-sky-500/10"
    } else if tool.kind == "edit" || tool.kind == "write" {
        "text-emerald-300/90 bg-emerald-500/10"
    } else if tool.kind == "read" {
        "text-amber-300/90 bg-amber-500/10"
    } else {
        "text-violet-300/90 bg-violet-500/10"
    };

    rsx! {
        div { class: "flex flex-col",
            div {
                class: "flex items-center gap-1.5 py-1 cursor-pointer select-none text-[11px] hover:bg-hover transition-colors rounded",
                onclick: move |e: MouseEvent| { e.stop_propagation(); is_open.set(!is_open()); },
                span { class: "{status_color} font-mono w-3.5 text-center shrink-0", "{status_char}" }
                span { class: "{kind_color} font-mono text-[9px] font-bold uppercase px-1 py-px rounded-sm shrink-0", "{tool.kind}" }
                span { class: "font-mono text-[11px] text-secondary flex-1 truncate min-w-0", "{tool.title}" }
                span { class: "text-[9px] text-muted shrink-0", if is_open() { "▾" } else { "▸" } }
            }
            if is_open() {
                div { class: "ml-3.5 mb-1.5 rounded bg-surface px-2.5 py-2",
                    if !tool.summary.is_empty() {
                        div { class: "text-[11px] text-secondary mb-1.5 leading-relaxed", "{tool.summary}" }
                    }
                    div { class: "font-mono text-[11px] leading-[1.5] text-secondary whitespace-pre-wrap break-all",
                        for line in tool.detail.lines() {
                            if line.starts_with('+') && !line.starts_with("+++") {
                                span { class: "text-success", "{line}\n" }
                            } else if line.starts_with('-') && !line.starts_with("---") {
                                span { class: "text-danger", "{line}\n" }
                            } else if line.starts_with("@@") {
                                span { class: "text-accent", "{line}\n" }
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

// 旧的分组/工具折叠面板已由 WorkProcessCollapsible / ProcessToolRow（无边框、暗色区分）替代。
