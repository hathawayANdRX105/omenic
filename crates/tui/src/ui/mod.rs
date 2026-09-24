//! ui/* — enhanced 外壳渲染（route §3：transcript 上、dock 下；T3 起加
//! 工具卡 / 问题面板 / footer 状态条）。
//!
//! 结构：[`layout`] 定竖向切分，[`transcript`] 出消息行（工具 part 交
//! [`tool_card`] 卡片化），[`footer`] 是状态条，[`questions`] 是 dock 上方
//! 的问题面板，[`dock`] 出活动/排队/提示，[`composer`] 出输入行与光标。
//! 样式全部来自 `crate::theme`（D11）：本目录不构造 `Color`，只消费语义 token。

mod composer;
mod dock;
pub mod footer;
mod layout;
pub mod panels;
pub mod questions;
pub mod session_picker;
pub mod tool_card;
mod transcript;

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::App;

/// 画一帧（自上而下）：transcript → footer 状态条 → 问题面板（无题 0 行）
/// → dock（活动 / 排队 / composer / 按键提示，恒压底——hints 仍是屏幕最
/// 底一行，T2 布局契约不变）。
///
/// footer 与面板的行从 transcript 底部让出：dock 几何仍由 [`layout::split`]
/// 原样决定，不动 T2 的切分语义（route §1 T3 白名单外的 layout/dock 零改动）。
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let (transcript_area, dock_area) = layout::split(area, app.queued().is_some());
    // footer：dock 之上恒 1 行（dock 压底时才有行可让）。
    let footer_height = u16::from(dock_area.y > 0);
    let footer_area = Rect {
        x: area.x,
        y: dock_area.y.saturating_sub(footer_height),
        width: area.width,
        height: footer_height,
    };
    // 问题面板：footer 之上、按 pending 实占行数让位（无题 = 0 行）。
    let panel_rows = app.questions().rows();
    let panel_y = footer_area.y.saturating_sub(panel_rows);
    let panel_area = Rect {
        x: area.x,
        y: panel_y,
        width: area.width,
        height: footer_area.y - panel_y,
    };
    // transcript：吃掉 footer + 面板让出的行。
    let transcript_area = Rect {
        height: panel_y.saturating_sub(transcript_area.y),
        ..transcript_area
    };
    transcript::render(frame, transcript_area, app);
    footer::render(frame, footer_area, app);
    app.questions().render(frame, panel_area);
    dock::render(frame, dock_area, app);
}
