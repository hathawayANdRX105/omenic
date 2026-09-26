//! ui/search_overlay — T11 转录搜索 overlay 渲染与命中行定位（route §3 T11）。
//!
//! 只渲染 + 定位原语：状态与键路由全在 [`SearchState`](crate::search::SearchState)
//! （`App` 持有，`ui::areas` 按
//! [`App::search_rows`](crate::app::App::search_rows)
//! 从 transcript 让行——渲染与让行同源，同斜杠面板口径）。
//!
//! [`line_offset] 把命中消息映射成 T6 视口坐标：行数**直接调**
//! `transcript::message_rows`（与渲染同一个函数、同一套分支口径，不存在
//! 第二份数法）；[`excerpt_spans`]
//! 按 [`match_ranges`](crate::search::match_ranges) 切高亮段（命中 = 唯一
//! 的高亮来源，样式走 `crate::theme`，D11 零字面色）。

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use web_state::types::ChatMessage;

use crate::app::App;
use crate::search::{self, SearchState};
use crate::theme;

/// 查询行提示符列宽（光标定位用，同 composer 的 `PROMPT` 口径）。
const PROMPT_COLS: u16 = 2;

/// excerpt 以首个命中为锚、命中前留的字符数（两侧开窗，长消息不整条铺开）。
const EXCERPT_BEFORE: usize = 16;

/// 画三行 overlay（`area` 已由 `ui::areas` 裁到 [`search::OVERLAY_ROWS`]）：
/// 查询行（提示符 + 词 + 跳转后的 `k/n`）→ 当前命中 excerpt（高亮段）→
/// 状态与按键提示；光标钉在查询词末尾（画在 dock 之后 = 焦点归 overlay）。
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let state = app.search_state();
    let mut lines = vec![
        query_line(state),
        hit_line(state, area.width),
        status_line(state),
    ];
    lines.truncate(area.height as usize);
    frame.render_widget(Paragraph::new(lines), area);
    let used = state.query().chars().count() as u16 + PROMPT_COLS;
    let col = used.min(area.width.saturating_sub(1));
    frame.set_cursor_position(Position::new(area.x + col, area.y));
}

/// 查询行：`> {query}` + 跳转后的 `{k}/{n}`（未跳转只显总命中数——还没
/// 定位过就报 `k/n` 是假精度）。
fn query_line(state: &SearchState) -> Line<'static> {
    let mut spans = vec![
        Span::styled("> ".to_string(), theme::brand_bold()),
        Span::styled(state.query().to_string(), theme::base()),
    ];
    if state.jumped() && !state.hits().is_empty() {
        spans.push(Span::styled(
            format!("  {}/{}", state.current() + 1, state.hits().len()),
            theme::dim(),
        ));
    }
    Line::from(spans)
}

/// 当前命中行：excerpt 高亮段；无命中 = 空行（提示在状态行，不编造结果）。
fn hit_line(state: &SearchState, width: u16) -> Line<'static> {
    match state.current_hit() {
        Some(hit) => Line::from(excerpt_spans(&hit.text, state.query(), width as usize)),
        None => Line::from(Vec::new()),
    }
}

/// 状态行：状态文案（错误 = danger，其余 dim）+ 恒定按键提示。
fn status_line(state: &SearchState) -> Line<'static> {
    let style = if state.error().is_some() {
        theme::danger()
    } else {
        theme::dim()
    };
    Line::from(vec![
        Span::styled(state.status(), style),
        Span::styled(
            " · enter next · shift+enter prev · esc restore",
            theme::dim(),
        ),
    ])
}

/// 当前命中的 excerpt spans：净化（滤控制符，transcript 同一口径）→
/// 换单行（换行压成空格，overlay 是单行预览）→ 以首个命中为锚开窗（左右
/// 省略号 `...`）→ 按 [`search::match_ranges`] 切段，命中段高亮。
/// 返回段拼接即展示文本（不含省略号的段外字符被开窗裁掉，Paragraph 列
/// 宽再裁一层——excerpt 是预览，不承诺全文）。
pub fn excerpt_spans(text: &str, query: &str, width: usize) -> Vec<Span<'static>> {
    let single = crate::ui::transcript::sanitize(text).replace('\n', " ");
    let ranges = search::match_ranges(&single, query);
    let chars: Vec<(usize, char)> = single.char_indices().collect();
    // 锚 = 首个命中（无命中回行首——正常不该发生，回执按 query 命中而来）。
    let anchor = ranges.first().map_or(0, |(start, _)| *start);
    let anchor_char = chars.partition_point(|(at, _)| *at < anchor);
    let budget = width.max(4);
    let from = anchor_char.saturating_sub(EXCERPT_BEFORE.min(budget / 2));
    let to = (from + budget).min(chars.len());

    let mut spans: Vec<Span<'static>> = Vec::new();
    if from > 0 {
        spans.push(Span::styled("...".to_string(), theme::dim()));
    }
    let mut group: Option<(bool, String)> = None;
    for (at, ch) in &chars[from..to] {
        let matched = ranges.iter().any(|(s, e)| at >= s && at < e);
        match &mut group {
            Some((was, buf)) if *was == matched => buf.push(*ch),
            _ => {
                if let Some((was, buf)) = group.take() {
                    spans.push(styled(was, buf));
                }
                group = Some((matched, ch.to_string()));
            }
        }
    }
    if let Some((was, buf)) = group.take() {
        spans.push(styled(was, buf));
    }
    if to < chars.len() {
        spans.push(Span::styled("...".to_string(), theme::dim()));
    }
    spans
}

/// 段样式：命中 = brand 加粗（唯一高亮来源），其余 = 正文。
fn styled(matched: bool, text: String) -> Span<'static> {
    Span::styled(
        text,
        if matched {
            theme::brand_bold()
        } else {
            theme::base()
        },
    )
}

/// 命中行定位：`up_to` 条消息之前的渲染行数——T6 视口坐标系里的目标行。
///
/// 行数 = `transcript::message_rows` 单一直接调用（渲染同源、无第二份
/// 实现；`tests/search_overlay.rs::line_offset_matches_transcript_total`
/// 仍用 `viewport().total()` 钉两边同和）。
pub fn line_offset(
    messages: &[ChatMessage],
    tools_expanded: bool,
    up_to: usize,
    width: u16,
) -> usize {
    let width = width.max(1) as usize;
    messages
        .iter()
        .take(up_to)
        .map(|msg| super::transcript::message_rows(msg, tools_expanded, width))
        .sum()
}
