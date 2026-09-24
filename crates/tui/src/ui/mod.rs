//! ui/* — enhanced 外壳渲染（route §3 T2：transcript 上、dock 下）。
//!
//! 结构切四块：[`layout`] 定竖向切分，[`transcript`] 出消息行，[`dock`]
//! 出活动/排队/提示，[`composer`] 出输入行与光标。样式全部来自
//! `crate::theme`（D11）：本目录不构造 `Color`，只消费语义 token。

mod composer;
mod dock;
mod layout;
mod transcript;

use ratatui::Frame;

use crate::app::App;

/// 画一帧：transcript 在上、dock 在下（composer + 活动 + 排队 + 按键提示）。
pub fn draw(frame: &mut Frame, app: &App) {
    let (transcript_area, dock_area) = layout::split(frame.area(), app.queued().is_some());
    transcript::render(frame, transcript_area, app);
    dock::render(frame, dock_area, app);
}
