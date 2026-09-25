//! ui/footer — 状态 footer（route §3 T3）。
//!
//! 运行中：`model · 耗时 · Esc 中断提示`；空闲：`model · run 状态`。
//! T6 起脱钩滚动时追加 `↑N 行` 段（距内容底的精确行数，回底消失）。
//!
//! **开工先核（route §2/§3 硬要求，核不到不许编）**：`stats.summary`
//! （`StatsSummary` + `unavailable` 清单）与消息侧都没有 per-context
//! 占用数据——run ledger 没有 token 列，`STATS_UNAVAILABLE` 明示
//! tokens/cost/model 等无持久化来源；web 的 `StatusLine.context_pct` 是
//! 页面本地启发值（分母 `DEFAULT_CONTEXT_MAX` 是 128k 中性常量，注释写明
//! 「真实 per-model 上限还没有数据源」）。核不到 → 空闲段只显示
//! `model · run 状态`（run 状态 = `stats.summary` 的半开 run 计数），
//! **不编造 context 百分比**（route §8「禁止编造 footer 数据」）。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::theme;

/// 字段分隔符。
const SEP: &str = " · ";

/// footer 的 `model` 段数据源：web 设置页同源的运行时配置（`.oi/config.toml`
/// 根 `model` / `[llm].model` / `OMENIC_LLM_MODEL`）。这是配置里的真实
/// 值，与 web 状态行取的是同一份配置文件，不是 footer 自己编的名字。
pub fn configured_model() -> String {
    web_client::llm::LlmRuntimeConfig::load_from_system().model
}

/// 渲染 footer 单行到 `area`（0 行区域直接跳过）。
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return;
    }
    frame.render_widget(Paragraph::new(line(app)), area);
}

/// footer 单行：已核字段用 ` · ` 串起，空段跳过。
pub fn line(app: &App) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut push = |text: String, style: Style| {
        if !spans.is_empty() {
            spans.push(Span::styled(SEP.to_string(), theme::dim()));
        }
        spans.push(Span::styled(text, style));
    };
    if !app.model().is_empty() {
        push(app.model().to_string(), theme::dim());
    }
    if app.is_running() {
        push(app.elapsed_label(), theme::brand());
        push("esc abort".to_string(), theme::brand_bold());
    } else {
        push(app.run_state_label(), theme::dim());
    }
    // T6：脱钩时的 `↑N 行` 指示（距内容底的精确行数；跟尾回底即消失，
    // route §3 T6）。空态（`None`）不出段，空闲段仍只含已核字段。
    if let Some(lift) = app.viewport().lift() {
        push(format!("↑{lift} 行"), theme::brand());
    }
    Line::from(spans)
}
