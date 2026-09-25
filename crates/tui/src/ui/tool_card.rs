//! ui/tool_card — 工具调用卡片（route §3 T3）。
//!
//! 数据源是投影好的 `UiState.messages[].parts[].Tool`（web-state 的
//! `ToolCall{title,kind,status}`）：一次 `ToolCall→ToolStart→ToolResult`
//! 序列在 `UiState::apply` 里折成**一个** `MessagePart::Tool`，本模块按
//! 「一个 part = 一张卡」渲染，**不重新实现归一化**——未知工具名的标题
//! 兜底由 `tool_call_from_rpc` 负责（`web/state/tests/ui_state.rs` 已钉）。
//!
//! 卡三态：running → done / failed（`ToolResult` 翻同一张卡的状态，不另开
//! 行）。长结果默认折叠成「头尾预览 + 隐藏行数」；展开状态由
//! `App::tools_expanded` 持有、一个键（Tab）全部展开/再按折回。

use ratatui::text::{Line, Span};

use web_state::types::ToolCall;

use crate::theme;

use super::transcript::push_wrapped;

/// 折叠阈值：结果超过该行数即默认折叠（route §3「长结果折叠」）。
pub const COLLAPSE_AFTER_LINES: usize = 6;
/// 折叠时保留的头部行数。
const HEAD_LINES: usize = 3;
/// 折叠时保留的尾部行数。
const TAIL_LINES: usize = 2;

/// 一张工具卡的全部行：卡头（kind · 三态）+ 标题 + 结果区。
///
/// `expanded` 由 `App::tools_expanded` 传入（一个键全部展开/折叠）；
/// `width` 是列宽，标题与结果按列宽断行（同 transcript 的处理）。
pub fn lines(tc: &ToolCall, expanded: bool, width: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    out.push(header(tc));
    push_wrapped(&mut out, &tc.title, width, "  ", theme::base());
    if tc.status == "running" {
        // 运行中 detail 还是入参 JSON、没有结果可看；结果由 ToolResult 落进
        // 这同一张卡——三态只在卡头翻，不新开行（route §3 一张卡契约）。
        return out;
    }
    out.extend(result_lines(tc, expanded, width));
    out
}

/// 卡头：`▸ <kind> · <三态>`，样式随状态（running=brand、failed=danger、
/// done=dim）。
fn header(tc: &ToolCall) -> Line<'static> {
    let style = match tc.status.as_str() {
        "running" => theme::brand_bold(),
        "error" => theme::danger(),
        _ => theme::dim(),
    };
    Line::from(Span::styled(
        format!("▸ {} · {}", tc.kind, status_label(&tc.status)),
        style,
    ))
}

/// 三态文案：success→done、error→failed；未知状态原样显示（只显示数据，
/// 不编造状态）。
fn status_label(status: &str) -> &str {
    match status {
        "success" => "done",
        "error" => "failed",
        other => other,
    }
}

/// 结果区：短结果全显；长结果默认「头尾预览 + 隐藏行数」，`expanded` 时
/// 全显（一个键全部展开/折叠）。
fn result_lines(tc: &ToolCall, expanded: bool, width: usize) -> Vec<Line<'static>> {
    let text = super::transcript::sanitize(&tc.detail);
    let logical: Vec<&str> = text.lines().collect();
    if logical.is_empty() {
        return Vec::new();
    }
    if expanded || logical.len() <= COLLAPSE_AFTER_LINES {
        let mut out = Vec::new();
        for line in logical {
            push_wrapped(&mut out, line, width, "  ", theme::base());
        }
        return out;
    }
    // 折叠：头 HEAD 行 + 隐藏行数 + 尾 TAIL 行（阈值恒大于头尾之和，
    // 切片不会越界）。
    let hidden = logical.len().saturating_sub(HEAD_LINES + TAIL_LINES);
    let mut out = Vec::new();
    for line in logical.iter().take(HEAD_LINES) {
        push_wrapped(&mut out, line, width, "  ", theme::base());
    }
    out.push(Line::from(Span::styled(
        format!("  … {hidden} lines hidden"),
        theme::dim(),
    )));
    for line in &logical[logical.len() - TAIL_LINES..] {
        push_wrapped(&mut out, line, width, "  ", theme::base());
    }
    out
}
