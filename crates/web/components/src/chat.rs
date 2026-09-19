//! 会话区（dsh ConversationRoot 复刻）：748px 消息列、用户右气泡 r22、
//! assistant 全宽 16/28、工作过程折叠行、浮动 composer（r22 胶囊卡）。

use dioxus::prelude::*;
use omenic_web_state::types::{ChatMessage, MessagePart, StatusLine, ToolCall};
use pulldown_cmark::{Options as MarkdownOptions, Parser, html};

use crate::icons::{ArrowUp, ChevronRight, Paperclip, SquareCheck};
use crate::ui::{Dropdown, IconButton, Spinner};

/// Markdown → HTML。CommonMark 会透传原始 HTML（经 `dangerous_inner_html`
/// 注入 DOM，是真实 XSS 面），所以生成后必须过 ammonia 白名单清洗：
/// 剥离 script/事件属性/javascript: URL，只留安全标签。
/// `pub` 供 `tests/markdown_sanitize.rs` 集成测试直接断言。
pub fn markdown_to_html(input: &str) -> String {
    let mut opts = MarkdownOptions::empty();
    opts.insert(MarkdownOptions::ENABLE_STRIKETHROUGH);
    opts.insert(MarkdownOptions::ENABLE_TASKLISTS);
    opts.insert(MarkdownOptions::ENABLE_TABLES);
    let parser = Parser::new_ext(input, opts);
    let mut output = String::new();
    html::push_html(&mut output, parser);
    ammonia::clean(&output)
}

/// 工具类型的 chip 配色（dsh 状态色 chip：900 底 + 400 字）。
///
/// `job` / `terminal` 复用 brand 家族：它们和 `bash` 一样是"跑命令"，
/// 换成另一种强调色会让同一类操作在气泡上显得互不相干。三者靠 chip 上的
/// kind 字符串（`job` / `terminal` / `bash`，见下方渲染处）区分，不靠颜色。
fn kind_chip(kind: &str) -> &'static str {
    match kind {
        "bash" | "job" | "terminal" => "bg-chip-brand text-brand-300",
        "edit" | "write" => "bg-chip-success text-success-2",
        "read" | "grep" | "glob" => "bg-chip-warn text-warn-2",
        "delete" => "bg-chip-danger text-danger",
        _ => "bg-layer-2 text-label-3",
    }
}

const MODELS: &[&str] = &[
    "deepseek-v4-flash",
    "qwen3-32b",
    "agnes-2.5-flash",
    "claude-opus-4-7",
    "kimi-k3",
];

const THINKING_OPTIONS: &[(&str, &str)] = &[
    ("关闭", "off"),
    ("轻量", "2k"),
    ("标准", "8k"),
    ("深度", "16k"),
];

#[component]
pub fn Chat(
    messages: Vec<ChatMessage>,
    statusline: StatusLine,
    is_streaming: bool,
    /// 浮在 composer 上方的 dock 卡片（任务看板等），由页面层传入
    dock: Option<Element>,
    on_send: EventHandler<String>,
    on_model_change: EventHandler<String>,
    on_toggle_thinking: EventHandler<()>,
    on_toggle_tasks: EventHandler<()>,
    /// 运行中点停止：中止当前 agent run
    on_abort: EventHandler<()>,
) -> Element {
    let mut draft = use_signal(String::new);
    let model_items: Vec<(String, String)> = MODELS
        .iter()
        .map(|m| (m.to_string(), m.to_string()))
        .collect();
    let thinking_items: Vec<(String, String)> = THINKING_OPTIONS
        .iter()
        .map(|(l, v)| (l.to_string(), v.to_string()))
        .collect();

    let display_messages: Vec<ChatMessage> = messages
        .iter()
        .filter(|m| {
            !m.content.is_empty() || !m.tool_calls.is_empty() || !m.parts.is_empty() || is_streaming
        })
        .cloned()
        .collect();

    let prompt_items: Vec<(String, String)> = display_messages
        .iter()
        .filter(|m| m.role == "user")
        .map(|m| (format!("prompt-{}", m.id), m.content.clone()))
        .collect();

    // 状态行耗时段（G5/5.6）：在飞 run 显示「当前时刻 - 开始时刻」，已结束
    // run 显示结算好的总耗时，两者都没有则为空串——空串时整段（含前导
    // 分隔符）不渲染，避免状态行出现 " · " 空档。rsx! 内禁止 let，故在
    // 此预先拼好。
    let elapsed = statusline.elapsed_label();
    let elapsed_seg = if elapsed.is_empty() {
        String::new()
    } else {
        format!(" · {elapsed}")
    };

    rsx! {
        div { class: "relative flex-1 min-h-0 overflow-hidden",
            // 单一滚动面板 = 整个聊天室
            div { class: "absolute inset-0 overflow-y-auto",
                id: "chat-scroll",
                div { class: "max-w-[780px] w-full mx-auto px-4 pt-4 pb-[220px] flex flex-col gap-4 min-h-full",
                    if display_messages.is_empty() && !is_streaming {
                        div { class: "flex-1 flex flex-col items-center justify-center gap-2.5 text-center py-10 select-none relative",
                            div { class: "absolute w-[520px] h-[220px] rounded-full bg-brand/10 blur-[110px] -z-10" }
                            div { class: "text-[26px] leading-8 font-semibold text-label", "开始一个新的任务" }
                            div { class: "text-[14px] leading-[22px] text-label-3 max-w-[420px]",
                                "在下方输入指令，Agent 将使用文件读写、bash 与代码编辑工具协助你完成。"
                            }
                        }
                    }
                    for (idx, msg) in display_messages.iter().enumerate() {
                        {
                            let is_last = idx == display_messages.len() - 1;
                            let anchor = if msg.role == "user" {
                                Some(format!("prompt-{}", msg.id))
                            } else {
                                None
                            };
                            rsx! {
                                MessageItem {
                                    key: "{msg.id}-{idx}",
                                    message: msg.clone(),
                                    // 流式指示只挂在最后一条 assistant 上
                                    streaming_tail: is_streaming && is_last && msg.role == "assistant",
                                    id: anchor,
                                }
                            }
                        }
                    }
                    div { id: "chat-scroll-anchor", class: "h-2 shrink-0" }
                }
            }

            // 左侧 minimap：每个用户 prompt 一个横条锚点（客户端 JS 聚光梯度）
            if !prompt_items.is_empty() {
                div { class: "absolute left-2 top-0 bottom-0 flex flex-col justify-center z-20",
                    id: "minimap",
                    for (anchor_id, p) in prompt_items.clone() {
                        div { class: "relative flex items-center",
                            style: "height:16px; width:56px;",
                            "data-anchor": anchor_id,
                            div { class: "rounded-full bg-subtle cursor-pointer transition-all duration-150 ease-out minimap-bar",
                                style: "height:4px; width:10px;",
                            }
                            div { class: "absolute left-14 top-1/2 -translate-y-1/2 z-30 w-[230px] max-h-[150px] overflow-hidden rounded-xl border border-binv bg-menu px-3 py-2.5 shadow-lv3 pointer-events-none",
                                "data-tip": "",
                                style: "display:none;",
                                div { class: "text-[12px] leading-5 text-label-2 whitespace-pre-wrap break-words line-clamp-6", "{p}" }
                            }
                        }
                    }
                }
            }

            // composer 悬浮座位：渐隐带 + 状态行 + dock + 输入卡
            div { class: "absolute bottom-0 left-0 right-0 z-30 pointer-events-none",
                div { class: "h-9 bg-gradient-to-t from-base to-transparent" }
                div { class: "mx-auto w-full max-w-[780px] px-4 pb-2 flex flex-col items-center gap-2",
                    // 状态行（dsh StatsLine：12/20 tertiary 居中）
                    div { class: "text-[12px] leading-5 text-label-3 text-center select-none",
                        "{statusline.model} · ↑{statusline.tokens_in} ↓{statusline.tokens_out} · ${statusline.cost_usd:.3} · context {statusline.context_pct:.0}%{elapsed_seg}"
                    }
                    // dock 卡片（任务看板）
                    {dock}
                    // 输入卡：r22 胶囊
                    // 不加 overflow-hidden：模型/思考菜单从工具行向上弹出，
                    // 裁剪会切掉卡片外的部分；圆角由卡片自身的 bg + radius 呈现
                    div { class: "pointer-events-auto w-full rounded-[22px] border border-b1 bg-input-bg shadow-lv2 flex flex-col transition-colors focus-within:border-b3",
                        textarea {
                            id: "chat-input-area",
                            class: "w-full resize-none bg-transparent border-none outline-none text-[16px] leading-6 text-label placeholder:text-caption caret-brand px-4 pt-3 pb-1 min-h-[52px] max-h-[336px]",
                            placeholder: "输入指令，Enter 发送，Shift+Enter 换行...",
                            value: "{draft}",
                            oninput: move |e: FormEvent| draft.set(e.value()),
                            onkeydown: move |e: KeyboardEvent| {
                                if e.key() == Key::Enter && !e.modifiers().contains(Modifiers::SHIFT) {
                                    e.prevent_default();
                                    let text = draft();
                                    if !text.trim().is_empty() && !is_streaming {
                                        on_send.call(text.trim().to_string());
                                        draft.set(String::new());
                                    }
                                }
                            },
                        }
                        div { class: "flex items-center justify-between pl-1.5 pr-2 pb-1.5 pt-0.5",
                            div { class: "flex items-center gap-0.5",
                                IconButton {
                                    title: "附件（待接线）",
                                    class: "bg-selector hover:bg-iactive",
                                    disabled: true,
                                    Paperclip { size: 15 }
                                }
                                IconButton {
                                    title: "任务看板",
                                    onclick: move |_| on_toggle_tasks.call(()),
                                    SquareCheck { size: 15 }
                                }
                                Dropdown {
                                    label: "{statusline.model}",
                                    header: "选择模型",
                                    items: model_items,
                                    active_value: statusline.model.clone(),
                                    mono: true,
                                    on_select: move |m: String| on_model_change.call(m),
                                }
                                Dropdown {
                                    label: "思考 {statusline.thinking}",
                                    header: "思考强度",
                                    items: thinking_items,
                                    active_value: statusline.thinking.clone(),
                                    on_select: move |_| on_toggle_thinking.call(()),
                                }
                            }
                            div { class: "flex items-center gap-2.5",
                                span { class: "text-[12px] leading-5 text-caption font-mono",
                                    "{statusline.tokens_in} / {statusline.tokens_out}"
                                }
                                // Send/Stop 同位切换（dsh：主按钮运行中即停止钮）
                                if is_streaming {
                                    button {
                                        r#type: "button",
                                        class: "w-[34px] h-[34px] rounded-full bg-brand text-white hover:bg-brand-hover flex items-center justify-center cursor-pointer transition-colors border-none",
                                        title: "停止",
                                        onclick: move |_| on_abort.call(()),
                                        span { class: "w-3 h-3 rounded-[2px] bg-white" }
                                    }
                                } else {
                                    button {
                                        r#type: "button",
                                        class: "w-[34px] h-[34px] rounded-full bg-brand text-white hover:bg-brand-hover flex items-center justify-center cursor-pointer transition-colors border-none",
                                        title: "发送",
                                        onclick: move |_| {
                                            let text = draft();
                                            if !text.trim().is_empty() && !is_streaming {
                                                on_send.call(text.trim().to_string());
                                                draft.set(String::new());
                                            }
                                        },
                                        ArrowUp { size: 16 }
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

/// 单条消息：用户右侧气泡 / assistant 按发生顺序（过程折叠 + 最终回复）。
#[component]
fn MessageItem(message: ChatMessage, streaming_tail: bool, id: Option<String>) -> Element {
    let is_user = message.role == "user";
    let dom_id = id.unwrap_or_default();

    if is_user {
        rsx! {
            div { class: "flex flex-col items-end gap-1 w-full group",
                id: "{dom_id}",
                if !message.content.is_empty() {
                    div { class: "markdown-sm bg-bubble rounded-[22px] px-4 py-2.5 max-w-[525px]",
                        dangerous_inner_html: "{markdown_to_html(&message.content)}"
                    }
                }
                span { class: "text-[12px] leading-5 text-label-3 pr-2 opacity-0 group-hover:opacity-100 transition-opacity duration-75",
                    "{message.timestamp}"
                }
            }
        }
    } else {
        // 有序片段：为空时回退 content + tool_calls
        let parts: Vec<MessagePart> = if !message.parts.is_empty() {
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
                let has_final = !ft.is_empty();
                (ft, parts[..fi].to_vec(), has_final)
            }
            None => (String::new(), parts.clone(), false),
        };
        let waiting = streaming_tail && !has_final && process.is_empty();

        rsx! {
            div { class: "flex flex-col gap-2 w-full group",
                id: "{dom_id}",
                if waiting {
                    // dsh turn 状态行：26px 高 shimmer
                    div { class: "h-[26px] flex items-center",
                        span { class: "shimmer-text text-[14px] font-medium", "思考中" }
                    }
                } else {
                    if !process.is_empty() {
                        ProcessBlock { parts: process, active: streaming_tail }
                    }
                    if has_final {
                        div { class: "markdown-body",
                            dangerous_inner_html: "{markdown_to_html(&final_text)}"
                        }
                    }
                    if streaming_tail {
                        div { class: "flex items-center gap-2 h-[26px]",
                            Spinner {}
                            span { class: "text-[12px] leading-5 text-label-3", "正在生成回复..." }
                        }
                    }
                    // hover 元信息（时间戳）
                    span { class: "text-[12px] leading-5 text-label-3 -ml-1 h-5 opacity-0 group-hover:opacity-100 transition-opacity duration-75",
                        "{message.timestamp}"
                    }
                }
            }
        }
    }
}

/// 「工作过程」折叠块：最终回复之前的全部内容（文本 + 工具调用）。
/// 运行中完全展开；最终回复到达后自动折叠。
#[component]
fn ProcessBlock(parts: Vec<MessagePart>, active: bool) -> Element {
    let mut is_open = use_signal(|| false);
    let open = active || is_open();
    let count = parts.len();

    rsx! {
        div { class: "flex flex-col",
            div { class: "h-6 flex items-center gap-1.5 cursor-pointer select-none w-fit text-[14px] leading-6 text-label-2 hover:text-label",
                onclick: move |e: MouseEvent| {
                    e.stop_propagation();
                    is_open.set(!is_open());
                },
                span { class: if open { "text-label-3 rotate-90 transition-transform duration-150" } else { "text-label-3 transition-transform duration-150" },
                    ChevronRight { size: 12 }
                }
                span { "工作过程" }
                span { class: "text-caption", "· {count}" }
            }
            if open {
                div { class: "pl-[22px] pt-1 flex flex-col gap-2",
                    for (i, p) in parts.iter().enumerate() {
                        match p {
                            MessagePart::Text(s) => rsx! {
                                div { key: "txt-{i}", class: "markdown-body",
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

/// 单次工具调用折叠行（dsh DisclosureRow：24px 头 + 展开体）。
#[component]
fn ToolLine(tool: ToolCall) -> Element {
    let mut open = use_signal(|| false);
    let is_err = tool.status == "error";
    let running = tool.status == "running";
    let chip = kind_chip(&tool.kind);
    let arrow_class = if open() {
        "text-label-3 transition-transform duration-150 rotate-90"
    } else {
        "text-label-3 transition-transform duration-150"
    };

    rsx! {
        div { class: "flex flex-col",
            div { class: "h-6 flex items-center gap-2 cursor-pointer select-none w-fit",
                onclick: move |e: MouseEvent| {
                    e.stop_propagation();
                    open.set(!open());
                },
                span { class: "{arrow_class}", ChevronRight { size: 12 } }
                span { class: "{chip} inline-flex items-center font-mono text-[10px] font-semibold uppercase px-1.5 py-px leading-4 rounded-md shrink-0",
                    "{tool.kind}"
                }
                span { class: "font-mono text-[12px] leading-5 text-label-2 flex-1 truncate min-w-0", "{tool.title}" }
                if running {
                    Spinner {}
                }
                if is_err {
                    span { class: "text-[11px] leading-4 font-medium text-danger shrink-0", "失败" }
                }
            }
            if open() {
                div { class: "pl-[22px] pb-1 flex flex-col gap-1.5",
                    if !tool.summary.is_empty() {
                        div { class: "text-[13px] leading-5 text-label-3", "{tool.summary}" }
                    }
                    div { class: "bg-codeblock rounded-lg px-3 py-2 font-mono text-[12px] leading-[18px] text-label-2 whitespace-pre-wrap break-all",
                        for line in tool.detail.lines() {
                            if line.starts_with('+') && !line.starts_with("+++") {
                                span { class: "text-success-2", "{line}\n" }
                            } else if line.starts_with('-') && !line.starts_with("---") {
                                span { class: "text-danger", "{line}\n" }
                            } else if line.starts_with("@@") {
                                span { class: "text-brand", "{line}\n" }
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
