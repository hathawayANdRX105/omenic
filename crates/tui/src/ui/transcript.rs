//! ui/transcript — `UiState` 消息 → 若干行（route §2 纯函数管线，D10）。
//!
//! 只读快照渲染：user 行带 brand 前缀、assistant 正文按 `parts` 顺序出
//! 文本/工具行（工具 part 交 [`super::tool_card`] 卡片化，T3）。按列宽
//! 自己断行，行数可数 → 底部对齐只需截尾，不依赖 Paragraph wrap 的不可见
//! 行数。模型内容先过 [`sanitize`]：裸控制字节不许进 cell（同 linear 零
//! ESC 约束）。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use web_state::types::MessagePart;

use crate::app::App;
use crate::theme;

/// 渲染 transcript 到 `area`（内容钉底：只显示最后能放下的行）。
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let lines = lines(app, area.width);
    let overflow = lines.len().saturating_sub(area.height as usize);
    frame.render_widget(
        Paragraph::new(lines.into_iter().skip(overflow).collect::<Vec<_>>()),
        area,
    );
}

/// 全部消息行（含被截掉的历史行；宽度用于断行）。
fn lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let width = width.max(1) as usize;
    let mut out = Vec::new();
    for msg in app.messages() {
        if msg.role == "user" {
            push_wrapped(&mut out, &msg.content, width, "❯ ", theme::brand_bold());
            continue;
        }
        if msg.parts.is_empty() {
            push_wrapped(&mut out, &msg.content, width, "", theme::base());
            continue;
        }
        for part in &msg.parts {
            match part {
                MessagePart::Text(text) => push_wrapped(&mut out, text, width, "", theme::base()),
                // T3：一个 Tool part = 一张卡（序列折叠与三态都在 tool_card）。
                MessagePart::Tool(tc) => {
                    out.extend(super::tool_card::lines(tc, app.tools_expanded(), width));
                }
            }
        }
    }
    out
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
    for (i, seg) in wrap(&clean, avail).into_iter().enumerate() {
        let head = if i == 0 { prefix } else { indent.as_str() };
        out.push(Line::from(Span::styled(format!("{head}{seg}"), style)));
    }
}

/// 按空白贪心断行；超宽单词按列宽硬切（不让 transcript 冲出列宽）。
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for raw in text.split('\n') {
        let mut line = String::new();
        for word in raw.split(' ') {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() <= width {
                line.push(' ');
                line.push_str(word);
                continue;
            }
            if !line.is_empty() {
                out.push(std::mem::take(&mut line));
            }
            let mut rest = word;
            while rest.chars().count() > width {
                let cut = rest
                    .char_indices()
                    .nth(width)
                    .map_or(rest.len(), |(i, _)| i);
                out.push(rest[..cut].to_string());
                rest = &rest[cut..];
            }
            line.push_str(rest);
        }
        out.push(line);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
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
