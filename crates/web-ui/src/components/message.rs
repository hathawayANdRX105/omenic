//! 消息树（自 chat.rs 拆出）：单条消息气泡、Work Process 折叠块、单次工具
//! 调用折叠行。三者只互相引用，故同处一文件。helper（kind_chip / kind_label /
//! format_tool_output / line_is_error / KindIcon）仍在 `super::chat`。

use dioxus::prelude::*;
use web_state::types::{ChatMessage, MessagePart, ToolCall, format_duration_ms};

use crate::shared as sh;
use ui_kit::Spinner;
use ui_kit::icons::{IconCheck, IconCopy};

use crate::utils::markdown::markdown_to_html;

use super::chat::{KindIcon, format_tool_output, kind_chip, kind_label, line_is_error};

/// 单条消息：用户右侧气泡 / assistant 按发生顺序（过程折叠 + 最终回复）。
#[component]
pub(crate) fn MessageItem(
    message: ChatMessage,
    streaming_tail: bool,
    id: Option<String>,
) -> Element {
    let is_user = message.role == "user";
    let dom_id = id.unwrap_or_default();

    if is_user {
        // 复制源：用户正文（纯图无正文时不显示复制钮）。
        let user_copy_src = message.content.clone();
        rsx! {
            div { class: "flex flex-col items-end gap-1 w-full group",
                id: "{dom_id}",
                // 用户带的图片：先图后气泡，与 freebuff 卡片顺序一致。
                if !message.attachments.is_empty() {
                    div { class: "flex flex-wrap justify-end gap-1.5 max-w-[525px]",
                        for att in message.attachments.iter() {
                            img {
                                src: "data:{att.media_type};base64,{att.data}",
                                alt: "{att.name}",
                                title: "{att.name}",
                                class: "max-h-[180px] rounded-[14px] border border-border object-cover",
                            }
                        }
                    }
                }
                if !message.content.is_empty() {
                    div { class: "markdown-sm bg-secondary rounded-[22px] px-4 py-2.5 max-w-[525px]",
                        dangerous_inner_html: "{markdown_to_html(&message.content)}"
                    }
                }
                div { class: "flex items-center gap-1.5 opacity-0 group-hover:opacity-100 transition-opacity duration-75 pr-2",
                    if !user_copy_src.is_empty() {
                        button {
                            class: "size-5 rounded flex items-center justify-center text-muted-foreground hover:text-foreground cursor-pointer border-none bg-transparent",
                            title: sh::BTN_COPY,
                            onclick: move |_| {
                                let t = user_copy_src.clone();
                                _ = document::eval(&format!(
                                    "navigator.clipboard && navigator.clipboard.writeText({t:?})"
                                ));
                            },
                            IconCopy { size: 11 }
                        }
                    }
                    span { class: "role-caption",
                        "{message.timestamp}"
                    }
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
                // 中间文本段不进过程（L2 只留思考段与工具行；最终答案单独成块）
                let process: Vec<MessagePart> = parts[..fi]
                    .iter()
                    .filter(|p| !matches!(p, MessagePart::Text(_)))
                    .cloned()
                    .collect();
                (ft, process, has_final)
            }
            None => (
                String::new(),
                parts
                    .iter()
                    .filter(|p| !matches!(p, MessagePart::Text(_)))
                    .cloned()
                    .collect(),
                false,
            ),
        };
        // reasoning 在流式时由各段 ReasoningBlock 自己的 shimmer 承担指示；
        // 「思考中」状态行只兜底「还没收到任何 reasoning/过程」的空窗期
        let waiting = streaming_tail && !has_final && process.is_empty();
        // aui ActionBar 复制源：最终正文优先，缺省回退整条 content
        let copy_src = if !final_text.is_empty() {
            final_text.clone()
        } else {
            message.content.clone()
        };

        rsx! {
            div { class: "flex flex-col gap-2 w-full group",
                id: "{dom_id}",
                if waiting {
                    // dsh turn 状态行：26px 高 shimmer
                    div { class: "h-[26px] flex items-center",
                        span { class: "shimmer-text role-hint font-medium text-foreground", {sh::MSG_THINKING} }
                    }
                } else {
                    // aui parts 模型：思考 / 工具 / 文本按真实发生顺序原位交替
                    // （ProcessBlock 内按序渲染 Reasoning 段 + ToolLine + 中间文本）
                    if !process.is_empty() {
                        ProcessBlock {
                            parts: process,
                            active: streaming_tail,
                            streaming: streaming_tail,
                        }
                    }
                    if has_final {
                        div { class: "markdown-body",
                            dangerous_inner_html: "{markdown_to_html(&final_text)}"
                        }
                    }
                    if streaming_tail {
                        div { class: "flex items-center gap-2 h-[26px]",
                            Spinner {}
                            span { class: "role-caption", {sh::MSG_GENERATING} }
                        }
                    }
                }
                // aui ActionBar（hideWhenRunning 对位）：生成全部结束后才出现
                if !streaming_tail {
                    div {
                        class: "flex items-center gap-2 -ml-1 h-5 opacity-0 group-hover:opacity-100 transition-opacity duration-75",
                        button {
                            class: "size-5 rounded flex items-center justify-center text-muted-foreground hover:text-foreground cursor-pointer",
                            title: sh::BTN_COPY,
                            onclick: move |_| {
                                let t = copy_src.clone();
                                _ = document::eval(&format!(
                                    "navigator.clipboard && navigator.clipboard.writeText({t:?})"
                                ));
                            },
                            IconCopy { size: 11 }
                        }
                        span { class: "role-caption", "{message.timestamp}" }
                    }
                }
            }
        }
    }
}
/// 「Work Process」折叠块：最终回复之前的过程项（思考段 / 工具行），按
/// 真实发生顺序原位渲染（aui parts 模型；中间文本段不进过程，上游已滤）。
/// 头行按状态换词（ainotation 波4 #1）：正在工作 = 动词形 `Progressing`
/// （shimmer），结束 = 名词形 `Progress`。默认状态跟随 `active`（运行中
/// 展开、跑完折叠，用户标注 2026-09-30 #2），用户点击头行可覆盖。
#[component]
fn ProcessBlock(parts: Vec<MessagePart>, active: bool, streaming: bool) -> Element {
    // None = 跟随 active；Some = 用户点过之后的显式开关
    let mut toggle = use_signal(|| None::<bool>);
    let open = toggle().unwrap_or(active);
    let count = parts.len();
    rsx! {
        div { class: "flex flex-col",
            div { class: "h-6 flex items-center gap-1.5 cursor-pointer select-none w-fit role-hint hover:text-foreground",
                onclick: move |e: MouseEvent| {
                    e.stop_propagation();
                    toggle.set(Some(!open));
                },
                span { class: "shrink-0", if streaming {
                    span { class: "shimmer-text", {sh::LBL_WORK_PROGRESSING} }
                } else {
                    {sh::LBL_WORK_PROGRESS}
                } }
                span { class: "text-muted-foreground", "· {count}" }
            }
            if open {
                div { class: "pl-[22px] pt-1 flex flex-col gap-2",
                    for (i, p) in parts.iter().enumerate() {
                        match p {
                            MessagePart::Tool(tc) => rsx! {
                                ToolLine { key: "{tc.id}-{i}", tool: tc.clone() }
                            },
                            // 上游已滤除（中间文本不进过程）；占位保持 match 穷尽
                            MessagePart::Text(_) => rsx! {
                                div { key: "txt-{i}", class: "hidden" }
                            },
                            MessagePart::Reasoning { text, duration_ms, .. } => {
                                let running = streaming && duration_ms.is_none();
                                rsx! {
                                    ReasoningBlock {
                                        key: "rsn-{i}",
                                        text: text.clone(),
                                        running,
                                        duration: *duration_ms,
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

/// 单次工具调用折叠行（dsh DisclosureRow：24px 头 + 展开体）。
/// 头行 = kind 图标 + 大写 kind 文本 + 「·」+ 命令标题（ainotation #4；
/// chevron 已按 #2 删除）。
#[component]
fn ToolLine(tool: ToolCall) -> Element {
    let mut open = use_signal(|| false);
    let is_err = tool.status == "error";
    let running = tool.status == "running";
    let chip = kind_chip(&tool.kind);
    let label = kind_label(&tool.kind);
    let formatted = format_tool_output(&tool.kind, &tool.detail);
    // aui ToolCall running 态：标题加 shimmer 扫光（CSS 变量，随现有主题走）
    let title_class = if running {
        "shimmer-text font-mono role-caption flex-1 truncate min-w-0"
    } else {
        "font-mono role-caption flex-1 truncate min-w-0"
    };

    rsx! {
        div { class: "flex flex-col",
            div { class: "h-6 flex items-center gap-2 cursor-pointer select-none w-full min-w-0",
                onclick: move |e: MouseEvent| {
                    e.stop_propagation();
                    open.set(!open());
                },
                KindIcon { kind: "{tool.kind}" }
                span { class: "{chip} font-mono role-overline shrink-0",
                    "{label}"
                }
                span { class: "role-caption shrink-0", "·" }
                // aui ToolCall running 态：标题 shimmer 替代 spinner 独占注意力
                span { class: "{title_class}", "{tool.title}" }
                if running {
                    Spinner {}
                }
                if is_err {
                    span { class: "role-label font-medium text-destructive shrink-0", {sh::MSG_TOOL_FAILED} }
                }
                if !running && !is_err {
                    // aui ToolCall 完成勾
                    IconCheck { size: 12, class: "shrink-0 text-success-foreground" }
                }
            }
            if open() {
                div { class: "pl-[22px] pb-1 flex flex-col gap-1.5",
                    if !tool.summary.is_empty() {
                        div { class: "role-caption", "{tool.summary}" }
                    }
                    div { class: "tool-output bg-codeblock rounded-lg px-3 py-2 font-mono role-caption whitespace-pre-wrap break-all",
                        for line in formatted.lines() {
                            if line.starts_with('+') && !line.starts_with("+++") {
                                span { class: "text-success-foreground", "{line}\n" }
                            } else if line.starts_with('-') && !line.starts_with("---") {
                                span { class: "text-destructive", "{line}\n" }
                            } else if line.starts_with("@@") {
                                span { class: "text-brand", "{line}\n" }
                            } else if line.starts_with('$') {
                                // 终端观感：命令行亮于输出行
                                span { class: "text-foreground", "{line}\n" }
                            } else if line_is_error(line) {
                                span { class: "text-destructive", "{line}\n" }
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

/// aui `Reasoning` part：受控折叠思考块，按发生顺序原位插在过程流里
/// （aui 分段语义：一段思考 = 一个块，工具/文本边界切段）。
///
/// 头行三态（ainotation 波4 #2）：
/// - 运行中：`Thinking · <思考正文最后完成行>` + Spinner。行随流推进不断
///   变化：换 `key` 触发节点重建，CSS `think-line` 动画每次换行播一次
///   （淡入 + 轻微上移）。
/// - 结算后：`Thought · Ns`（aui "Thought for Ns" 词族；无真实耗时回退
///   `Thought`）。
/// 默认**始终折叠**（运行时不自动展开，用户点击整行打开；
/// #2 的无 chevron 决策）。思考正文走 markdown 渲染，左侧边框保留「过程
/// 显示」观感。
///
/// 「最后完成行」口径：只有到达换行符的行才算完成（当前未收尾的行仍在
/// 生长，不进头行）；取最后一段非空行，截断到 80 字符（`truncate` 双保险）。
fn last_completed_line(text: &str) -> Option<String> {
    let pos = text.rfind('\n')?;
    text[..pos].lines().rev().find_map(|l| {
        let t = l.trim();
        (!t.is_empty()).then(|| {
            let s: String = t.chars().take(80).collect();
            if t.chars().count() > 80 {
                format!("{s}…")
            } else {
                s
            }
        })
    })
}

#[component]
fn ReasoningBlock(text: String, running: bool, duration: Option<u64>) -> Element {
    // 默认折叠（运行时也不自动展开）；Some = 用户点过之后的显式开关
    let mut toggle = use_signal(|| None::<bool>);
    let open = toggle().unwrap_or_default();
    let done_label: String = match duration {
        Some(ms) if ms > 0 => format!("{} · {}", sh::LBL_THOUGHT, format_duration_ms(ms)),
        _ => sh::LBL_THOUGHT.to_string(),
    };
    let line = last_completed_line(&text);
    rsx! {
        div { class: "select-none",
            div {
                class: "h-5 flex items-center gap-1.5 cursor-pointer w-full min-w-0",
                onclick: move |e: MouseEvent| {
                    e.stop_propagation();
                    toggle.set(Some(!open));
                },
                if running {
                    span { class: "shimmer-text role-caption font-medium shrink-0", {sh::LBL_THINKING_EN} }
                    if let Some(l) = &line {
                        // key 随「最后完成行」内容变 → 节点重建 → think-line
                        // 换行动画每次播一次
                        span { key: "{l}", class: "think-line role-caption flex-1 min-w-0 truncate", "· {l}" }
                    }
                    span { class: "ml-1 shrink-0", Spinner {} }
                } else {
                    span { class: "role-caption hover:text-foreground transition-colors", "{done_label}" }
                }
            }
            if open {
                div {
                    class: "mt-1 markdown-sm border-l border-border pl-3",
                    dangerous_inner_html: "{markdown_to_html(&text)}"
                }
            }
        }
    }
}
