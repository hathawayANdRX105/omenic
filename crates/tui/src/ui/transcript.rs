//! ui/transcript — `UiState` 消息 → 若干行（route §2 纯函数管线，D10）。
//!
//! 只读快照渲染：user 行带 brand 前缀、assistant 正文按 `parts` 顺序出
//! 文本/工具行（工具 part 交 [`super::tool_card`] 卡片化，T3）。按列宽
//! 自己断行，行数可数 → 底部对齐只需截尾，不依赖 Paragraph wrap 的不可见
//! 行数。模型内容先过 [`sanitize`]：裸控制字节不许进 cell（同 linear 零
//! ESC 约束）。
//!
//! **T6 窗口化**：渲染不再物化全部历史行——第一遍 [`total_lines`] 只
//! **计数**（与物化共用 [`wrap_with`] 同一断行核心，历史行不生成
//! `Line`/`Span`），第二遍 [`window`] 只物化与视口窗口
//! `[view_top, view_top + height)` 相交的消息。视口首行来自
//! [`ScrollModel`](crate::scroll::ScrollModel)（翻页 / 脱钩 / 跟尾滑动都
//! 在模型里，渲染只读）。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use web_state::types::{ChatMessage, MessagePart};

use crate::app::App;
use crate::theme;

/// 渲染 transcript 到 `area`：窗口化——只物化可见窗口（route §3 T6 契约
/// 种子），历史行不重复物化。
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let width = area.width.max(1);
    let height = area.height as usize;
    let total = total_lines(app, width);
    let top = app.viewport().view_top(total, height);
    frame.render_widget(
        Paragraph::new(window(app, width as usize, top, height)),
        area,
    );
}

/// 内容总行数（第一遍：**只计数不物化**——与 [`window`] 共用断行核心，
/// 分叉由 `tests/scroll_follow.rs::window_render_matches_line_count` 钉）。
pub(super) fn total_lines(app: &App, width: u16) -> usize {
    let width = width.max(1) as usize;
    app.messages()
        .iter()
        .map(|msg| message_rows(app, msg, width))
        .sum()
}

/// 一条消息的行数（与 [`push_message`] 逐分支同源，分叉即测试红）。
fn message_rows(app: &App, msg: &ChatMessage, width: usize) -> usize {
    if msg.role == "user" {
        return count_wrapped(&msg.content, width, "❯ ");
    }
    if msg.parts.is_empty() {
        return count_wrapped(&msg.content, width, "");
    }
    msg.parts
        .iter()
        .map(|part| match part {
            MessagePart::Text(text) => count_wrapped(text, width, ""),
            MessagePart::Tool(tc) => super::tool_card::rows(tc, app.tools_expanded(), width),
        })
        .sum()
}

/// 可见窗口的行（第二遍）：只物化与 `[top, top + height)` 相交的消息，
/// 再切片到窗口——其上（更旧）与其下（更新）的消息一行为都不生成。
fn window(app: &App, width: usize, top: usize, height: usize) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::with_capacity(height);
    let mut start = 0usize;
    for msg in app.messages() {
        let rows = message_rows(app, msg, width);
        let end = start + rows;
        if end <= top {
            // 整条在窗口之上（更旧）：跳过，不物化。
            start = end;
            continue;
        }
        if start >= top + height {
            break; // 整条在窗口之下（更新）：其后只会更靠下，收工。
        }
        if rows > 0 {
            let mut buf: Vec<Line<'static>> = Vec::with_capacity(rows);
            push_message(&mut buf, app, msg, width);
            let from = top.saturating_sub(start);
            let to = (top + height).min(end) - start;
            out.extend(buf.into_iter().skip(from).take(to - from));
        }
        start = end;
        if out.len() >= height {
            break;
        }
    }
    out
}

/// 一条消息 → 若干行（与 [`message_rows`] 同一数据路径的物化侧）。
fn push_message(out: &mut Vec<Line<'static>>, app: &App, msg: &ChatMessage, width: usize) {
    if msg.role == "user" {
        push_wrapped(out, &msg.content, width, "❯ ", theme::brand_bold());
        return;
    }
    if msg.parts.is_empty() {
        push_wrapped(out, &msg.content, width, "", theme::base());
        return;
    }
    for part in &msg.parts {
        match part {
            MessagePart::Text(text) => push_wrapped(out, text, width, "", theme::base()),
            // T3：一个 Tool part = 一张卡（序列折叠与三态都在 tool_card）。
            MessagePart::Tool(tc) => {
                out.extend(super::tool_card::lines(tc, app.tools_expanded(), width));
            }
        }
    }
}

/// 一段文本 → 若干行：首行带 `prefix`，续行补同样宽的空白；样式统一。
/// （`pub(super)`：`tool_card` 的标题/结果区复用同一断行与清洗。）
pub(super) fn push_wrapped(
    out: &mut Vec<Line<'static>>,
    text: &str,
    width: usize,
    prefix: &str,
    style: Style,
) {
    let clean = sanitize(text);
    let indent = " ".repeat(prefix.chars().count());
    let avail = width.saturating_sub(prefix.chars().count()).max(1);
    let head = prefix;
    let mut line = 0usize;
    wrap_with(&clean, avail, |seg| {
        // 首行前缀 / 续行空白由调用次序决定（第 0 段 = 首行）。
        let head = if line == 0 { head } else { indent.as_str() };
        line += 1;
        out.push(Line::from(Span::styled(format!("{head}{seg}"), style)));
    });
}

/// 一段文本 → 行数（[`push_wrapped`] 的计数侧，断行核心同一份——窗口化
/// 第一遍只数不物化）。`pub(super)`：`tool_card` 的标题/结果区计数复用。
pub(super) fn count_wrapped(text: &str, width: usize, prefix: &str) -> usize {
    let clean = sanitize(text);
    let avail = width.saturating_sub(prefix.chars().count()).max(1);
    let mut rows = 0usize;
    wrap_with(&clean, avail, |_| rows += 1);
    rows
}

/// 断行核心：按空白贪心断行、超宽单词按列宽硬切，逐段回调（不持有段、
/// 不物化 `Line`——计数侧与物化侧共用这一份，杜绝两边算法漂移）。
fn wrap_with(text: &str, width: usize, mut f: impl FnMut(&str)) {
    let width = width.max(1);
    for raw in text.split('\n') {
        let mut line = String::new();
        for word in raw.split(' ') {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() <= width {
                line.push(' ');
                line.push_str(word);
                continue;
            }
            if !line.is_empty() {
                let taken = std::mem::take(&mut line);
                f(&taken);
            }
            let mut rest = word;
            while rest.chars().count() > width {
                let cut = rest
                    .char_indices()
                    .nth(width)
                    .map_or(rest.len(), |(i, _)| i);
                f(&rest[..cut]);
                rest = &rest[cut..];
            }
            line.push_str(rest);
        }
        f(&line);
    }
}

/// 滤 C0 控制符：制表并为空格、其余（含 ESC 字节）丢掉，保留换行。
/// 按字符过滤不会劈开 UTF-8 多字节序列；控制字节进 cell 会被后端原样
/// 写回终端，必须在渲染前掐掉（同 `linear.rs` 的零 ESC 契约）。
pub(super) fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| if c == '\t' { ' ' } else { c })
        .filter(|c| !c.is_ascii_control() || *c == '\n')
        .collect()
}
