//! ui/dock — 底部 dock：当前活动 + 排队 prompt + composer + 按键提示
//! （route §3 T2 dock 四件套；不做 inline dock / 坐标锚定，decisions D5 修订）。

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::theme;

use super::composer;
use super::layout::dock_rows;

/// 行序：活动 / [排队] / composer / 按键提示（恒在最底一行）。
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let rows = dock_rows(app.queued().is_some());
    let chunks = Layout::vertical(vec![Constraint::Length(1); rows as usize]).split(area);
    frame.render_widget(Paragraph::new(activity(app)), chunks[0]);
    if app.queued().is_some() {
        frame.render_widget(Paragraph::new(queue_line(app.queued_count())), chunks[1]);
    }
    composer::render(frame, chunks[chunks.len() - 2], app);
    frame.render_widget(Paragraph::new(hints(app)), chunks[chunks.len() - 1]);
}

/// 当前活动：错误/中断覆写（danger）→ 运行中（brand）→ 空闲（dim）。
fn activity(app: &App) -> Line<'static> {
    if app.status_text().is_empty() {
        if app.is_running() {
            return Line::from(Span::styled("● running", theme::brand_bold()));
        }
        return Line::from(Span::styled("○ idle", theme::dim()));
    }
    Line::from(Span::styled(
        format!("● {}", app.status_text()),
        theme::danger(),
    ))
}

/// 排队计数（route §3 T10 `queued: n`，n = 队列条数：入队/召回/消费三点
/// 都走 [`App::queued_count`]）。只显计数不预览正文——dh-rs dock 的
/// `Next turn queued | n items` 同口径（看内容用首行 ↑ 召回）。
fn queue_line(count: usize) -> Line<'static> {
    Line::from(vec![
        Span::styled("queued: ", theme::brand_bold()),
        Span::styled(count.to_string(), theme::dim()),
    ])
}

/// 按键提示（随状态换行文，恒压在 dock 最底、≤44 列不截断）。
fn hints(app: &App) -> Line<'static> {
    if app.confirm_quit() {
        return Line::from(Span::styled(
            "esc again to quit · other key cancels",
            theme::danger(),
        ));
    }
    let text = if app.is_running() {
        "enter queue · ↑↓ history · esc abort"
    } else {
        "enter send · ↑↓ history · esc quit · ctrl-d"
    };
    Line::from(Span::styled(text, theme::dim()))
}
