//! ui/layout — enhanced 外壳的竖向切分（route §3 T2 布局契约：
//! 上方 transcript 滚动区 + 底部 dock，不做 inline dock / scrollback 锚定）。

use ratatui::layout::{Constraint, Layout, Rect};

/// dock 基础行数：活动 1 + composer 1 + 按键提示 1。
pub const BASE_DOCK_ROWS: u16 = 3;

/// dock 行数：有排队 prompt 再占 1 行（route §3 dock 四件套）。
pub fn dock_rows(has_queue: bool) -> u16 {
    BASE_DOCK_ROWS + u16::from(has_queue)
}

/// 把整屏切成 `(transcript, dock)`。
///
/// `Constraint::Min(1)` 给 transcript 保底一行，dock 恒拿 [`dock_rows`] 行
/// ——44×20 下 composer 也挤不掉（route §4 `dock_layout_fits` 钉的语义）。
pub fn split(area: Rect, has_queue: bool) -> (Rect, Rect) {
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(dock_rows(has_queue))])
        .split(area);
    (chunks[0], chunks[1])
}
