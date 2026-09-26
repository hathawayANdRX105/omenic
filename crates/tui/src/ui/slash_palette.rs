//! ui/slash_palette — T9 斜杠命令候选面板（route §3 T9：命令名 + 一行
//! 描述，模糊过滤）。
//!
//! 只渲染：面板状态（可见性 / 过滤 / 游标 / 抑制）全在 [`App`]，这里读
//! [`App::slash_matches`] / [`App::slash_selected`] 与行数的
//! [`App::slash_rows`] 同源——`ui::areas` 按那个数从 transcript 让行，
//! 渲染与让行不会漂移。样式全部走 `crate::theme`（D11，无 ESC 字节）。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::theme;

/// 画候选到 `area`（行数由 `areas` 裁好：0 行 = 面板关闭，直接返回；
/// 无命中给 1 行提示，行数超出则截尾）。
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return;
    }
    let selected = app.slash_selected();
    let matches = app.slash_matches();
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(matches.len().max(1));
    if matches.is_empty() {
        lines.push(Line::styled("no matching command", theme::dim()));
    } else {
        for cmd in matches {
            let highlighted = selected.is_some_and(|sel| sel.name == cmd.name);
            lines.push(Line::from(vec![
                Span::styled(
                    if highlighted { "> " } else { "  " }.to_string(),
                    theme::brand_bold(),
                ),
                Span::styled(
                    cmd.name.to_string(),
                    if highlighted {
                        theme::brand_bold()
                    } else {
                        theme::base()
                    },
                ),
                Span::styled(format!("  {}", cmd.description), theme::dim()),
            ]));
        }
    }
    lines.truncate(area.height as usize);
    frame.render_widget(Paragraph::new(lines), area);
}
