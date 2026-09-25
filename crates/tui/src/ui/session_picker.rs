//! ui/session_picker — 会话导航 picker（route §3 T4）。
//!
//! 三层切分（route §2 纯函数管线）：[`PickerState`] 是无 IO 纯状态（按键
//! 进去、[`PickerAction`] 出来），[`render`] 把状态画进 ratatui 帧，
//! [`run_picker`] 是真终端驱动（取数 → 画 → 读键 → 按需重查）。
//!
//! 行为契约（route §3 T4，签名不许改）：
//! - `filter` 起步：空词 = `list_sessions` 全量列表；
//! - 输入即按新词 `search_sessions` 过滤（空词回落全量）；
//! - Enter 返回选中 session id；ESC 返回 `None`（调用方保持原会话）；
//! - 列表为空 → 显示「无会话」单行，Enter 不选中、不崩；
//! - 每行渲染 `runs_for_session` 最新 run 的 Active/Aborted 徽章；
//! - 选中后的历史回填走 [`load_history`]（`load_messages`），回填进
//!   [`crate::app::App::switch_session`] 时一并设滚动边界。

use std::collections::HashMap;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use web_client::daemon::WebDaemon;
use web_state::types::{ChatMessage, Session};

use crate::termguard::{CrosstermOps, TermGuard};
use crate::theme;
use crate::{TuiError, client_error};

/// 起步/过滤的会话条数上限（picker 是导航，不是数据导出）。
const LIST_LIMIT: u32 = 50;
/// 每个会话取多少条 run 记录判 Active/Aborted 徽章。
const RUN_LIMIT: u32 = 200;
/// 选中后历史回填条数上限（滚动边界以回填结果为准）。
const HISTORY_LIMIT: u32 = 500;
/// 驱动层按键轮询间隔（每次轮询前先画，键来了即刻重画）。
const POLL: Duration = Duration::from_millis(50);

/// 会话行的 run 徽章（route §3 T4：渲染 `runs_for_session` 的
/// Active/Aborted；正常收尾的 run 不挂徽章）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunBadge {
    /// 最新 run 未收尾（在飞）。
    Active,
    /// 最新 run 被显式中止。
    Aborted,
}

/// picker 的一行：会话 + run 徽章。
#[derive(Debug, Clone, PartialEq)]
pub struct PickerItem {
    pub session: Session,
    pub badge: Option<RunBadge>,
}

/// 一次按键的结论（取数/重查由驱动层执行，状态机保持纯）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerAction {
    /// 无事发生（含空列表上的 Enter——「无会话」单行不崩）。
    None,
    /// 过滤词变了：驱动层按新词 `search_sessions` 后 [`PickerState::set_items`]
    ///（空词 = 全量列表）。
    Refetch(String),
    /// Enter 选中：携带该行的 session id。
    Select(String),
    /// ESC：返回原会话（驱动层映射成 `Ok(None)`）。
    Cancel,
}

/// picker 纯状态：过滤词 + 结果行 + 高亮游标（无 IO，可被测试直接驱动）。
#[derive(Debug, Clone, PartialEq)]
pub struct PickerState {
    filter: String,
    items: Vec<PickerItem>,
    cursor: usize,
}

impl PickerState {
    /// 起步状态（`filter` 即调用方给的初始过滤词，空词 = 全量）。
    pub fn new(filter: &str, items: Vec<PickerItem>) -> Self {
        Self {
            filter: filter.to_string(),
            items,
            cursor: 0,
        }
    }

    /// 当前过滤词。
    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// 当前结果行。
    pub fn items(&self) -> &[PickerItem] {
        &self.items
    }

    /// 高亮行下标（空列表恒 0，[`Self::selected`] 返回 `None`）。
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// 高亮行（空列表 = `None`，Enter 因此不选中、不崩）。
    pub fn selected(&self) -> Option<&PickerItem> {
        self.items.get(self.cursor)
    }

    /// 换入重查结果（`search_sessions` 回执）：游标回顶，不越界。
    pub fn set_items(&mut self, items: Vec<PickerItem>) {
        self.items = items;
        self.cursor = 0;
    }

    /// 一次按键路由（非 Press/Repeat 忽略；↑↓ 夹紧在列表范围内）。
    pub fn handle_key(&mut self, key: KeyEvent) -> PickerAction {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return PickerAction::None;
        }
        match key.code {
            KeyCode::Esc => PickerAction::Cancel,
            KeyCode::Enter => self
                .selected()
                .map(|item| PickerAction::Select(item.session.id.clone()))
                .unwrap_or(PickerAction::None),
            KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                PickerAction::None
            }
            KeyCode::Down => {
                if !self.items.is_empty() {
                    self.cursor = (self.cursor + 1).min(self.items.len() - 1);
                }
                PickerAction::None
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.filter.push(c);
                PickerAction::Refetch(self.filter.clone())
            }
            KeyCode::Backspace => {
                self.filter.pop();
                PickerAction::Refetch(self.filter.clone())
            }
            _ => PickerAction::None,
        }
    }
}

/// 画一帧 picker：过滤行 + 结果列表 + 按键提示（光标钉在过滤词末尾）。
pub fn render(frame: &mut Frame, state: &PickerState) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(frame.area());
    frame.render_widget(Paragraph::new(filter_line(state)), chunks[0]);
    render_list(frame, chunks[1], state);
    frame.render_widget(Paragraph::new(hints_line()), chunks[2]);
    // 光标落在 "> " 之后的过滤词末尾（窄屏夹进可视区）。
    let used = state.filter().chars().count() as u16 + PROMPT_COLS;
    let col = used.min(chunks[0].width.saturating_sub(1));
    frame.set_cursor_position(Position::new(chunks[0].x + col, chunks[0].y));
}

/// 过滤行的 "> " 前缀列宽（光标定位用）。
const PROMPT_COLS: u16 = 2;

/// 过滤行：提示符 + 过滤词 + 结果计数（dim）。
fn filter_line(state: &PickerState) -> Line<'static> {
    Line::from(vec![
        Span::styled("> ".to_string(), theme::brand_bold()),
        Span::styled(state.filter().to_string(), theme::base()),
        Span::styled(format!("  ({})", state.items().len()), theme::dim()),
    ])
}

/// 结果列表：空 → 「无会话」单行；非空 → 按高亮游标取可视窗口。
fn render_list(frame: &mut Frame, area: ratatui::layout::Rect, state: &PickerState) {
    if state.items().is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled("无会话", theme::dim()))),
            area,
        );
        return;
    }
    let height = area.height.max(1) as usize;
    // 视窗贴住高亮行（长列表只显示能放下的尾段，不越界）。
    let scroll = state.cursor().saturating_sub(height.saturating_sub(1));
    let rows: Vec<Line> = state
        .items()
        .iter()
        .enumerate()
        .skip(scroll)
        .take(height)
        .map(|(i, item)| row_line(i == state.cursor(), item))
        .collect();
    frame.render_widget(Paragraph::new(rows), area);
}

/// 一行会话：游标前缀 + Active/Aborted 徽章 + 标题（空标题回退 id）。
fn row_line(selected: bool, item: &PickerItem) -> Line<'static> {
    let mut spans = vec![Span::styled(
        if selected { "> " } else { "  " },
        if selected {
            theme::brand_bold()
        } else {
            theme::dim()
        },
    )];
    if let Some(badge) = item.badge {
        let (label, style) = match badge {
            RunBadge::Active => ("[Active] ", theme::brand_bold()),
            RunBadge::Aborted => ("[Aborted] ", theme::danger()),
        };
        spans.push(Span::styled(label.to_string(), style));
    }
    let title = if item.session.title.is_empty() {
        item.session.id.clone()
    } else {
        item.session.title.clone()
    };
    spans.push(Span::styled(
        title,
        if selected {
            theme::brand()
        } else {
            theme::base()
        },
    ));
    Line::from(spans)
}

/// 按键提示行（恒在最底一行）。
fn hints_line() -> Line<'static> {
    Line::from(Span::styled(
        "enter select · esc cancel · ↑↓ move · type to filter",
        theme::dim(),
    ))
}

/// 选中后的历史回填数据源（route §3 T4：`load_messages`）。
pub fn load_history(client: &WebDaemon, sid: &str) -> Result<Vec<ChatMessage>, TuiError> {
    client
        .load_messages(sid, HISTORY_LIMIT)
        .map_err(client_error)
}

/// 会话导航 picker（route §3 T4 签名，不许改）。
///
/// 行为契约见模块文档。终端进出：raw 已开（enhanced 外壳内）= 调用方管
/// 终端，本函数只接管绘制、退出不碰终端状态；raw 未开（独立调用）经
/// [`TermGuard`] 进 alternate screen + raw，退出/panic 都还原。
pub fn run_picker(client: &WebDaemon, filter: &str) -> Result<Option<String>, TuiError> {
    let mut badges: HashMap<String, Option<RunBadge>> = HashMap::new();
    let mut state = PickerState::new(filter, fetch_items(client, filter, &mut badges)?);
    // 嵌套判定：raw 已开 = 调用方（run_enhanced 的 TermGuard）管终端。
    let nested = crossterm::terminal::is_raw_mode_enabled().unwrap_or(false);
    let mut guard = TermGuard::new(CrosstermOps);
    if !nested {
        // 独立调用：panic 先还原再打印（同 run_enhanced 的底线）。
        crate::termguard::install_panic_hook();
        if let Err(e) = guard.enter() {
            let _ = guard.leave();
            return Err(TuiError::Io(e));
        }
    }
    let picked = picker_loop(client, &mut state, &mut badges);
    let leave = guard.leave();
    match (picked, leave) {
        (Err(err), _) => Err(err),
        (Ok(_), Err(err)) => Err(TuiError::Io(err)),
        (Ok(value), Ok(())) => Ok(value),
    }
}

/// 驱动循环：画 → 读键 → [`PickerAction`] 分派（重查只发生在
/// [`PickerAction::Refetch`] 上，每次按键一查、空词回落全量）。
fn picker_loop(
    client: &WebDaemon,
    state: &mut PickerState,
    badges: &mut HashMap<String, Option<RunBadge>>,
) -> Result<Option<String>, TuiError> {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(std::io::stdout()))?;
    loop {
        terminal.draw(|frame| render(frame, state))?;
        if !event::poll(POLL)? {
            continue;
        }
        // resize：下一轮 draw 的 autoresize 自动重排；paste/focus 不改状态。
        match event::read()? {
            Event::Key(key) => match state.handle_key(key) {
                PickerAction::None => {}
                PickerAction::Cancel => return Ok(None),
                PickerAction::Select(id) => return Ok(Some(id)),
                PickerAction::Refetch(query) => {
                    state.set_items(fetch_items(client, &query, badges)?);
                }
            },
            _ => {}
        }
    }
}

/// 按词取会话行 + 每行 run 徽章。
///
/// 空词 = `list_sessions` 全量起步；非空 = `search_sessions` 过滤。
/// 徽章按会话缓存在驱动层（`runs_for_session` 是整本 ledger 的客户端过滤，
/// 每次按键重查全部会话会退化成 N 次全文件读）。
fn fetch_items(
    client: &WebDaemon,
    filter: &str,
    badges: &mut HashMap<String, Option<RunBadge>>,
) -> Result<Vec<PickerItem>, TuiError> {
    let sessions = if filter.trim().is_empty() {
        client.list_sessions(LIST_LIMIT).map_err(client_error)?
    } else {
        client
            .search_sessions(filter, LIST_LIMIT)
            .map_err(client_error)?
    };
    Ok(sessions
        .into_iter()
        .map(|session| {
            let badge = match badges.get(&session.id) {
                Some(cached) => *cached,
                None => {
                    let badge = run_badge(client, &session.id);
                    badges.insert(session.id.clone(), badge);
                    badge
                }
            };
            PickerItem { session, badge }
        })
        .collect())
}

/// 最新 run 的徽章：半开（`finished_at_ms` 为空）→ Active；`aborted` →
/// Aborted；正常收尾 / 无 run / RPC 失败 → 无徽章（徽章是装饰，不许把
/// picker 拖死）。
fn run_badge(client: &WebDaemon, sid: &str) -> Option<RunBadge> {
    let runs = client.runs_for_session(sid, RUN_LIMIT).ok()?;
    let latest = runs.iter().max_by_key(|run| run.seq)?;
    if latest.finished_at_ms.is_none() {
        return Some(RunBadge::Active);
    }
    if latest.status.as_deref() == Some("aborted") {
        return Some(RunBadge::Aborted);
    }
    None
}
