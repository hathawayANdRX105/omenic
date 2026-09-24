//! ui/composer — dock 的输入行（route §3 四件套之一：非空 Enter 提交、
//! ↑/↓ 走本地历史；编辑语义全在 `app::App`，这里只负责显示与光标）。

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::theme;

/// composer 提示符（transcript 的用户前缀是 `❯`，这里用 `>` 区分）。
const PROMPT: &str = "> ";

/// 渲染输入行并把光标钉在插入点。
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let prompt_cols = PROMPT.chars().count();
    // append-only 编辑：输入超宽时横向滚动，只留末尾能放下的部分
    //（视窗永远贴着插入点，长输入不会把行撑出 dock）。
    let visible = (area.width as usize).saturating_sub(prompt_cols).max(1);
    let window: String = app
        .input()
        .chars()
        .rev()
        .take(visible)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let line = Line::from(vec![
        Span::styled(PROMPT, theme::brand_bold()),
        Span::styled(window, theme::base()),
    ]);
    frame.render_widget(Paragraph::new(line), area);
    // 光标位置夹进可视区（窄终端下不许写到 dock 之外）。
    let used = app.input().chars().count().min(visible) as u16;
    let col = (prompt_cols as u16 + used).min(area.width.saturating_sub(1));
    frame.set_cursor_position(Position::new(area.x + col, area.y));
}
