use crate::components::ui::Dropdown;
use crate::mock::{ChatMessage, MessagePart, StatusLine, ToolCall};
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

/// 不同工具类型对应的暗色徽标配色（无边框，靠颜色区分）。
fn kind_badge(kind: &str) -> &'static str {
    match kind {
        "bash" => "text-sky-300/90 bg-sky-500/10",
        "edit" | "write" => "text-emerald-300/90 bg-emerald-500/10",
        "read" => "text-amber-300/90 bg-amber-500/10",
        _ => "text-violet-300/90 bg-violet-500/10",
    }
}

/// 缩略导航横条：静止时全部等宽一致；悬停时由客户端 JS 按鼠标位置
/// 生成高斯“聚光”梯度（光标处最长最亮，向两端平滑收窄）。

#[component]
pub fn Chat(
    messages: Vec<ChatMessage>,
    statusline: StatusLine,
    is_streaming: bool,
    on_send: EventHandler<String>,
    on_model_change: EventHandler<String>,
    on_toggle_thinking: EventHandler<()>,
) -> Element {
    let models = [
        "agnes-2.5-flash",
        "deepseek-v4-flash",
        "claude-opus-4-7",
        "qwen3-32b",
        "kimi-k3",
    ];

    let thinking_options: Vec<(String, String)> = vec![
        ("关闭".into(), "off".into()),
        ("轻量".into(), "2k".into()),
        ("标准".into(), "8k".into()),
        ("深度".into(), "16k".into()),
    ];

    let display_messages: Vec<_> = messages
        .iter()
        .filter(|m| {
            !m.content.is_empty() || !m.tool_calls.is_empty() || !m.parts.is_empty() || is_streaming
        })
        .cloned()
        .collect();

    let last_idx = display_messages.len().saturating_sub(1);

    let prompt_items: Vec<(String, String)> = display_messages
        .iter()
        .filter(|m| m.role == "user")
        .map(|m| (format!("prompt-{}", m.id), m.content.clone()))
        .collect();

    rsx! {
        div { class: "relative flex-1 min-h-0 overflow-hidden",
            // 单一滚动面板 = 整个聊天室（只有一个滚动条，位于房间最右侧）
            div { class: "absolute inset-0 overflow-y-auto",
                id: "chat-scroll",
                div { class: "max-w-[1360px] w-[96%] mx-auto px-6 pt-5 pb-[160px] flex flex-col gap-3.5 min-h-full",
                if display_messages.is_empty() && !is_streaming {
                    div { class: "flex-1 flex flex-col items-center justify-center gap-2 text-muted text-center py-10 select-none",
                        div { class: "text-base font-semibold text-muted-foreground", "开始一个新的任务" }
                        div { class: "text-xs max-w-[420px] leading-relaxed", "在下方输入指令，Agent 将使用文件读写、bash 与代码编辑工具协助你完成。" }
                    }
                }
                for (idx, msg) in display_messages.iter().enumerate() {
                    // 用户的一次输入 + agent 的完整回答 = 一个完整过程；过程之间用清晰分界线隔开。
                    if msg.role == "user" && idx > 0 {
                        div { class: "h-px w-full bg-border-hover my-3 shrink-0" }
                    }
                    if msg.content.is_empty() && msg.tool_calls.is_empty() && msg.parts.is_empty() {
                        div { key: "streaming-{idx}", class: "flex items-center gap-2 py-1 text-[12px] text-muted",
                            dioxus_components::Spinner { size: dioxus_components::SpinnerSize::Small }
                            span { "正在连接模型并思考生成回答..." }
                        }
                    } else {
                        MessageBubble { key: "{msg.id}-{idx}", message: msg.clone(), active: is_streaming && idx == last_idx && msg.role == "assistant", id: if msg.role == "user" { Some(format!("prompt-{}", msg.id)) } else { None } }
                    }
                }
                div { id: "chat-scroll-anchor", class: "h-4 shrink-0" }
                }
            }

            // 左侧缩略导航：每个用户 prompt 一个高亮横条作为锚点；悬停显示该次 prompt 的缩略面板。
            // 整列垂直居中（中心向两边扩展），新增时重新居中。
            if !prompt_items.is_empty() {
                div { class: "absolute left-2 top-0 bottom-0 flex flex-col justify-center z-20 pointer-events-auto",
                    id: "minimap",
                    for (anchor_id, p) in prompt_items.clone().into_iter() {
                        div { class: "relative flex items-center",
                            style: "height:16px; width:56px;",
                            "data-anchor": anchor_id.clone(),
                            div { class: "rounded-full bg-subtle cursor-pointer transition-all duration-150 ease-out minimap-bar",
                                style: "height:4px; width:12px;",
                            }
                            div { class: "absolute left-14 top-1/2 -translate-y-1/2 z-30 w-[230px] max-h-[150px] overflow-hidden rounded-lg border border-subtle bg-surface-elevated px-3 py-2.5 shadow-[0_8px_26px_rgba(0,0,0,0.42)] pointer-events-none",
                                "data-tip": "",
                                style: "display:none;",
                                div { class: "text-[11px] leading-relaxed text-muted-foreground whitespace-pre-wrap break-words line-clamp-6", "{p}" }
                            }
                        }
                    }
                }
            }

            // 输入框悬浮在聊天室上方（pointer-events-none 让滚动穿透，仅输入框本身可交互）
            div { class: "absolute bottom-0 left-1/2 -translate-x-1/2 w-[95%] max-w-[1260px] px-5 pb-4 z-30 pointer-events-none",
                div { class: "bg-surface border border-subtle rounded-xl shadow-[0_8px_26px_rgba(0,0,0,0.42)] flex flex-col transition-all focus-within:border-accent pointer-events-auto",
                    form {
                        class: "flex flex-col",
                        onsubmit: move |e: FormEvent| {
                            let text = e
                                .get_first("message")
                                .and_then(|v| match v {
                                    FormValue::Text(s) => Some(s.trim().to_string()),
                                    _ => None,
                                })
                                .unwrap_or_default();
                            if !text.is_empty() && !is_streaming {
                                on_send.call(text);
                            }
                        },

                        textarea {
                            id: "chat-input-area",
                            name: "message",
                            class: "w-full min-h-[68px] max-h-[220px] bg-transparent border-none outline-none text-foreground font-normal text-[13px] leading-[1.55] px-4 pt-3.5 pb-2 resize-none placeholder:text-muted",
                            placeholder: "输入指令，Enter 发送，Shift+Enter 换行...",
                        }

                        div { class: "flex items-center justify-between px-3 pt-2 pb-2.5 border-t border-[rgba(255,255,255,0.04)]",
                            div { class: "flex items-center gap-1",
                                // 模型下拉（复用 Dropdown，自带外点收起）
                                Dropdown {
                                    label: "{statusline.model} ▾",
                                    header: "选择模型",
                                    items: models.iter().map(|m| (m.to_string(), m.to_string())).collect(),
                                    active_value: statusline.model.clone(),
                                    mono: true,
                                    on_select: move |m: String| {
                                        on_model_change.call(m);
                                    },
                                }
                                // 思考下拉（复用 Dropdown，自带外点收起）
                                Dropdown {
                                    label: "思考 {statusline.thinking} ▾",
                                    header: "思考强度",
                                    items: thinking_options.clone(),
                                    active_value: statusline.thinking.clone(),
                                    on_select: move |_v: String| {
                                        on_toggle_thinking.call(());
                                    },
                                }
                            }

                            div { class: "flex items-center gap-2",
                                span { class: "text-[10px] text-muted font-mono", "{statusline.tokens_in} / {statusline.tokens_out}" }
                                button {
                                    r#type: "submit",
                                    class: if is_streaming { "flex items-center gap-1.5 px-3.5 py-1.5 rounded-md text-xs font-semibold bg-accent text-[#0b0c10] opacity-75 cursor-not-allowed" } else { "flex items-center gap-1.5 px-3.5 py-1.5 rounded-md text-xs font-semibold bg-accent text-[#0b0c10] hover:bg-accent-hover cursor-pointer transition-colors" },
                                    disabled: is_streaming,
                                    if is_streaming {
                                        dioxus_components::Spinner { size: dioxus_components::SpinnerSize::Small }
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
fn MessageBubble(message: ChatMessage, active: bool, id: Option<String>) -> Element {
    let is_user = message.role == "user";

    if is_user {
        // 用户消息：右侧消息气泡（无表头，模拟 zcode 风格）
        rsx! {
            div { class: "flex flex-col items-end gap-1 w-full",
                id: id.clone().unwrap_or_default(),
                if !message.content.is_empty() {
                    div { class: "markdown-body text-foreground bg-surface-elevated border border-accent-subtle rounded-[10px] px-3 py-2 text-[13px] leading-[1.55] max-w-[82%]",
                        dangerous_inner_html: "{markdown_to_html(&message.content)}"
                    }
                }
            }
        }
    } else {
        // 助手消息：按真实发生顺序渲染。
        // 最后一段文本作为「最终回复」始终展示；其之前的内容（中间输出 + 工具调用）
        // 收进单个「过程」折叠块——运行时完全展开，最终回复到达后自动折叠。
        // 只有真正的 reason chunk 才算「思考」，当前数据无此类内容。
        let parts = if !message.parts.is_empty() {
            message.parts.clone()
        } else {
            let mut v = Vec::new();
            for tc in &message.tool_calls {
                v.push(MessagePart::Tool(tc.clone()));
            }
            if !message.content.is_empty() {
                v.push(MessagePart::Text(message.content.clone()));
            }
            v
        };

        let final_idx = parts
            .iter()
            .rposition(|p| matches!(p, MessagePart::Text(_)));
        let (final_text, process, has_final) = match final_idx {
            Some(fi) => {
                let ft = match &parts[fi] {
                    MessagePart::Text(s) => s.clone(),
                    _ => String::new(),
                };
                (ft.clone(), parts[..fi].to_vec(), !ft.is_empty())
            }
            None => (String::new(), parts.clone(), false),
        };

        rsx! {
            div { class: "flex flex-col gap-2 w-full",
                id: id.clone().unwrap_or_default(),
                if !process.is_empty() {
                    ProcessBlock { parts: process, active }
                }
                if has_final {
                    div { class: "markdown-body text-foreground",
                        dangerous_inner_html: "{markdown_to_html(&final_text)}"
                    }
                }
                if active {
                    div { class: "flex items-center gap-2 py-1 text-[12px] text-muted",
                                        dioxus_components::Spinner { size: dioxus_components::SpinnerSize::Small }
                        span { "正在生成回复..." }
                    }
                }
            }
        }
    }
}

/// 单个折叠块：包裹「最终回复」之前的所有内容（中间输出 + 工具调用）。
/// 运行时（active）完全展开；最终回复到达后自动折叠，仅留最终回复可见。
#[component]
fn ProcessBlock(parts: Vec<MessagePart>, active: bool) -> Element {
    let mut is_open = use_signal(|| false);
    let open = active || is_open();
    let count = parts.len();

    rsx! {
        div { class: "flex flex-col",
            div {
                class: "flex items-center gap-1.5 py-0.5 cursor-pointer select-none text-[11px] rounded hover:text-muted-foreground",
                onclick: move |e: MouseEvent| { e.stop_propagation(); is_open.set(!is_open()); },
                span { class: "font-medium text-muted", "过程" }
                span { class: "text-muted/70", " · {count}" }
            }
            if open {
                div { class: "flex flex-col gap-2 pl-1 mt-0.5",
                    for (i, p) in parts.iter().enumerate() {
                        match p {
                            MessagePart::Text(s) => rsx! {
                                div { key: "txt-{i}", class: "markdown-body text-foreground/90",
                                    dangerous_inner_html: "{markdown_to_html(s)}"
                                }
                            },
                            MessagePart::Tool(tc) => rsx! {
                                ToolLine { key: "{tc.id}-{i}", tool: tc.clone() }
                            },
                        }
                    }
                }
            }
        }
    }
}

/// 单次工具调用：在「过程」块内作为可折叠项展示（默认折叠，仅显示一行表头）。
/// 点击表头展开其 summary + 完整输出；执行失败（status == "error"）时以红色「失败」标记提醒。
#[component]
fn ToolLine(tool: ToolCall) -> Element {
    let mut open = use_signal(|| false);
    let is_err = tool.status == "error";
    let badge = if is_err {
        "text-danger bg-danger/10"
    } else {
        kind_badge(&tool.kind)
    };
    let line_color = if is_err {
        "border-danger/40"
    } else {
        "border-subtle/40"
    };
    let arrow = if open() { "▾" } else { "▸" };

    rsx! {
        div { class: "flex flex-col",
            div { class: "flex items-center gap-1.5 py-1 text-[11px] rounded cursor-pointer select-none hover:text-muted-foreground",
                onclick: move |e: MouseEvent| { e.stop_propagation(); open.set(!open()); },
                span { class: "text-[10px] text-muted/70 w-3 text-center shrink-0", "{arrow}" }
                span { class: "{badge} inline-flex items-center justify-center font-mono text-[9px] font-bold uppercase px-1.5 py-0.5 leading-none rounded-sm shrink-0", "{tool.kind}" }
                if is_err {
                    span { class: "text-danger font-mono text-[9px] font-bold uppercase px-1 py-px rounded-sm shrink-0", "失败" }
                }
                span { class: "font-mono text-[11px] text-muted-foreground flex-1 truncate min-w-0", "{tool.title}" }
            }
            if open() {
                div { class: "ml-3.5 mb-1.5 border-l-2 {line_color} pl-3 py-0.5",
                    if !tool.summary.is_empty() {
                        div { class: "text-[11px] text-muted mb-1.5 leading-relaxed", "{tool.summary}" }
                    }
                    div { class: "font-mono text-[11px] leading-[1.5] text-muted-foreground/70 whitespace-pre-wrap break-all",
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
