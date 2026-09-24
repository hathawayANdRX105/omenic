//! linear 输出投影：`AgentEvent` → 裸文本行（route §3 线性渲染器）。
//!
//! T1 全量走本模块：无 alternate screen、无 raw mode、无光标寻址，事件
//! 按行裸写 stdout（route §8 禁止项的安全档）。[`render_linear_line`] 是
//! 全 T1 唯一的 [`UiState::apply`] 调用点：每事件先 apply 拿最新态再
//! 投影，`TurnEnd` 收行——不为「行开着」缓存字节，stdout 无未冲刷残留。
//!
//! 零 ESC 字节是硬契约（route §3/§8）：模型内容不可信，[`sanitize`] 把
//! ESC、回车、响铃等 C0 控制字符滤掉（保留换行与制表），渲染器自己也不
//! 写任何转义序列——piped smoke 的输出永远是纯文本。

use omenic_web_state::types::MessagePart;
use omenic_web_state::ui_state::{AgentEvent, UiState};

/// 一条事件的线性投影（route §3 签名，不许改：ev 先于 state）。
///
/// - `AssistantText`：delta 原样累写、不换行（`TurnEnd` 收行）；
/// - `Reasoning`：只进 `state.reasoning` 不回显——推理可见性属 T3 卡片，
///   transcript 只留正文与工具行（对齐 dsh-rs ChatState 只吃 TextDelta）；
/// - `ToolCall` / `ToolResult`：整行 `[tool] …`（文本行开着则先收行）；
/// - `ToolStart`：只 apply（running 态），无正文；
/// - `TurnEnd`：空 turn 把 [`UiState`] 刚写入的占位正文补显，行恒以换行
///   收尾。
pub fn render_linear_line(
    ev: &AgentEvent,
    state: &mut UiState,
    out: &mut impl std::io::Write,
) -> std::io::Result<()> {
    // apply 会就地改状态，两件事必须先取快照：
    // - TurnEnd 的空 turn 判定（apply 的 TurnEnd 臂可能当场写占位正文）；
    // - 文本行是否开着（工具行打断前先收行，别黏在正文尾巴上）。
    let was_empty_turn = matches!(ev, AgentEvent::TurnEnd { .. }) && empty_turn(state);
    let break_before = matches!(
        ev,
        AgentEvent::ToolCall { .. } | AgentEvent::ToolResult { .. }
    ) && text_line_open(state);
    state.apply(ev);
    match ev {
        AgentEvent::TurnStart => {}
        // 文本增量：原样累写、不换行。
        AgentEvent::AssistantText { delta } => write_clean(out, delta)?,
        // 推理增量不回显（apply 已累积进 state.reasoning）。
        AgentEvent::Reasoning { .. } => {}
        // 工具调用：整行 [tool] {name}: {title}，title 与 name 相同则省略。
        AgentEvent::ToolCall { name, id, .. } => {
            if break_before {
                writeln!(out)?;
            }
            let title = tool_title(state, id).unwrap_or_else(|| name.clone());
            let line = if title == *name {
                format!("[tool] {name}")
            } else {
                format!("[tool] {name}: {title}")
            };
            writeln!(out, "{}", sanitize(&line))?;
        }
        // 启动帧无正文：等带标题的 ToolCall（或结果）落行。
        AgentEvent::ToolStart { .. } => {}
        // 工具结果：整行 [tool] {name}: {summary}。
        AgentEvent::ToolResult { id, name, .. } => {
            if break_before {
                writeln!(out)?;
            }
            let summary = tool_summary(state, id);
            writeln!(out, "{}", sanitize(&format!("[tool] {name}: {summary}")))?;
        }
        // TurnEnd：空 turn 补显占位正文，行恒收尾换行。
        AgentEvent::TurnEnd { .. } => {
            if was_empty_turn {
                let placeholder = state
                    .messages
                    .last()
                    .map(|m| m.content.as_str())
                    .unwrap_or("");
                if !placeholder.is_empty() {
                    write_clean(out, placeholder)?;
                }
            }
            writeln!(out)?;
        }
    }
    Ok(())
}

/// 空 turn：最后一条不是「有正文或有工具的 assistant」。判定条件逐字对齐
/// `UiState::apply` 的 TurnEnd 臂（content 与 tool_calls 皆空才写占位），
/// 两边不能漂，否则会出现「state 写了占位、屏幕没显」或反过来。
fn empty_turn(state: &UiState) -> bool {
    match state.messages.last() {
        Some(msg) if msg.role == "assistant" => msg.content.is_empty() && msg.tool_calls.is_empty(),
        _ => true,
    }
}

/// 文本行是否开着（最后一条消息的最后一个片段是文本）。
fn text_line_open(state: &UiState) -> bool {
    matches!(
        state.messages.last().and_then(|m| m.parts.last()),
        Some(MessagePart::Text(_))
    )
}

/// 最近一条 assistant 消息（apply 的目标消息）。
fn last_assistant(state: &UiState) -> Option<&omenic_web_state::types::ChatMessage> {
    state.messages.iter().rev().find(|m| m.role == "assistant")
}

/// 指定 id 工具的展示标题：`tool_call_from_rpc` 的派生态（bash 取 command、
/// read/edit 取 path，未知名字回落工具名本身）。
fn tool_title(state: &UiState, id: &str) -> Option<String> {
    last_assistant(state)?
        .tool_calls
        .iter()
        .find(|t| t.id == id)
        .map(|t| t.title.clone())
}

/// 指定 id 工具的执行态文案：apply 的 ToolResult 臂写入「执行完成/执行
/// 失败」；结果没配上调用 → 兜底「已结束」。
fn tool_summary(state: &UiState, id: &str) -> String {
    last_assistant(state)
        .and_then(|m| m.tool_calls.iter().find(|t| t.id == id))
        .map(|t| t.summary.clone())
        .unwrap_or_else(|| "已结束".to_string())
}

/// 滤掉控制字符（ESC、回车、响铃等 C0 与 DEL），只留换行与制表。
/// 按字符过滤且只删 ASCII 控制符，不会劈开 UTF-8 多字节序列。
fn sanitize(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_ascii_control() || *c == '\n' || *c == '\t')
        .collect()
}

/// 唯一落盘口：写入净化后的文本，保证输出零 ESC 字节。
fn write_clean(out: &mut impl std::io::Write, text: &str) -> std::io::Result<()> {
    out.write_all(sanitize(text).as_bytes())
}
