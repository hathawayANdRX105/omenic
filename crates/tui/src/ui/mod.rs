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
pub mod search_overlay;
pub mod session_picker;
mod slash_palette;
pub mod tool_card;
mod transcript;

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::App;

/// 一帧的区域几何（T2 竖向切分 + T3 让位 + T9 斜杠面板让行 + T11 搜索
/// overlay 让行）。[`draw`] 与 [`sync_viewport`] 共用同一计算——滚动模型
/// 与渲染看到的 transcript 视口永远是同一套，窗口不会漂移。
struct Areas {
    transcript: Rect,
    footer: Rect,
    panel: Rect,
    palette: Rect,
    search: Rect,
    dock: Rect,
}

/// 把整屏切成 transcript / 斜杠面板 / 搜索 overlay / footer / 问题面板 /
/// dock 六块（语义与 T2/T3/T9/T11 逐条一致，抽出来给两条调用方共用）。
fn areas(app: &App, area: Rect) -> Areas {
    let (transcript, dock) = layout::split(area, app.queued().is_some());
    // footer：dock 之上恒 1 行（dock 压底时才有行可让）。
    let footer_height = u16::from(dock.y > 0);
    let footer = Rect {
        x: area.x,
        y: dock.y.saturating_sub(footer_height),
        width: area.width,
        height: footer_height,
    };
    // 问题面板：footer 之上、按 pending 实占行数让位（无题 = 0 行）。
    let panel_rows = app.questions().rows();
    let panel_y = footer.y.saturating_sub(panel_rows);
    let panel = Rect {
        x: area.x,
        y: panel_y,
        width: area.width,
        height: footer.y - panel_y,
    };
    // T11 搜索 overlay：问题面板之上恒 `search::OVERLAY_ROWS` 行（0 行 =
    // 关闭；小屏按 transcript 余量夹紧）。与斜杠面板开合互斥（`slash_visible`
    // 在 overlay 打开时恒 false），两块不会同帧同时占行。
    let search_rows = app.search_rows().min(panel_y.saturating_sub(transcript.y));
    let search_y = panel_y.saturating_sub(search_rows);
    let search = Rect {
        x: area.x,
        y: search_y,
        width: area.width,
        height: panel_y - search_y,
    };
    // T9 斜杠面板：问题面板之上、同样从 transcript 让行（0 行 = 关闭；
    // 行数 = 面板候选数，无命中也留 1 行提示；小屏按 transcript 余量夹紧）。
    let slash_rows = app.slash_rows().min(search_y.saturating_sub(transcript.y));
    let slash_y = search_y.saturating_sub(slash_rows);
    let palette = Rect {
        x: area.x,
        y: slash_y,
        width: area.width,
        height: search_y - slash_y,
    };
    // transcript：吃掉斜杠面板 + 搜索 overlay + footer + 面板让出的行。
    let transcript = Rect {
        height: slash_y.saturating_sub(transcript.y),
        ..transcript
    };
    Areas {
        transcript,
        footer,
        panel,
        palette,
        search,
        dock,
    }
}

/// 画一帧（自上而下）：transcript → 斜杠面板（T9，0 行不画）→ footer 状态条
/// → 问题面板（无题 0 行）→ dock（活动 / 排队 / composer / 按键提示，恒压底
/// ——hints 仍是屏幕最底一行，T2 布局契约不变）→ 搜索 overlay（T11，0 行
/// 不画）。**最后一笔**是 overlay：光标钉在查询行，压过 composer 的光标
/// （同一帧只留一个 cursor，焦点归打开的一方）。
///
/// footer 与两层面板的行从 transcript 底部让出：dock 几何仍由 [`layout::split`]
/// 原样决定，不动 T2 的切分语义（route §1 T3 白名单外的 layout/dock 零改动）。
pub fn draw(frame: &mut Frame, app: &App) {
    let areas = areas(app, frame.area());
    transcript::render(frame, areas.transcript, app);
    slash_palette::render(frame, areas.palette, app);
    footer::render(frame, areas.footer, app);
    app.questions().render(frame, areas.panel);
    dock::render(frame, areas.dock, app);
    search_overlay::render(frame, areas.search, app);
}

/// T6：本帧滚动几何同步——把 transcript 视口行数与内容总行数喂给
/// [`ScrollModel::sync`](crate::scroll::ScrollModel::sync)（跟尾滑动 /
/// clamp 在模型里推进，渲染只读模型）。事件循环在每次 `draw` 前调用；
/// 契约测试用同一入口驱动整帧，不绕开模型自己算边界。
///
/// T11 顺带记下 transcript 列宽（[`App::set_view_width`]）——命中行定位
/// 按同一份断行几何算，与 `transcript::total_lines` 同源。
pub fn sync_viewport(app: &mut App, screen: Rect) {
    let areas = areas(app, screen);
    let total = transcript::total_lines(app, areas.transcript.width);
    app.set_view_width(areas.transcript.width);
    app.viewport_mut()
        .sync(total, areas.transcript.height as usize);
}
