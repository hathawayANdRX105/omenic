//! ui/panels — 任务面板：dock 上方只读渲染 `task_list` / `todo_list` /
//! `goal_list`（route §3 T4）。
//!
//! **只读展示**：TUI 零写路径——todo/goal 的写方是模型侧工具、task 的写方
//! 是 `oi task` CLI（handoff F3），本模块连按键都不接。
//!
//! 渲染位置由调用方（`app.rs` 的 draw 接缝）决定：[`render`] 自己按
//! [`layout::split`] 算出可见 transcript 区（扣除 T3 问题面板 + footer
//! 两层让行）、贴其底部铺行，dock 之上、不改 [`super::draw`] 的既有布局。

use omenic_web_client::daemon::WebDaemon;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::theme;

use super::layout;

/// 三个列表各取多少条（看板式点缀，不是数据导出）。
const PANEL_LIMIT: u32 = 50;
/// 每节最多渲染几行（外加 1 行节头；三节全满 = 12 行封顶）。
const MAX_ROWS_PER_SECTION: usize = 3;

/// 面板一行：标题 + 状态标签（`{Debug}` 形态，如 `InProgress` / `Done`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelRow {
    pub title: String,
    pub status: String,
}

/// 任务面板快照：三张表的只读投影。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PanelSnapshot {
    pub tasks: Vec<PanelRow>,
    pub todos: Vec<PanelRow>,
    pub goals: Vec<PanelRow>,
}

impl PanelSnapshot {
    /// 三节全空 = 面板不占任何行（[`render`] 直接返回）。
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty() && self.todos.is_empty() && self.goals.is_empty()
    }
}

/// 拉三张表组快照（`task_list` / `todo_list` / `goal_list`）。
///
/// 单表 RPC 失败按空表算：面板是点缀，不许把聊天主链拖死（daemon 断线
/// 由事件流的收线信号负责报退出码 3，这里不重复报）。
pub fn load_panels(client: &WebDaemon) -> PanelSnapshot {
    let tasks = client.task_list(PANEL_LIMIT).unwrap_or_default();
    let todos = client.todo_list(PANEL_LIMIT).unwrap_or_default();
    let goals = client.goal_list(PANEL_LIMIT).unwrap_or_default();
    PanelSnapshot {
        tasks: tasks
            .into_iter()
            .map(|task| PanelRow {
                title: task.title,
                status: format!("{:?}", task.status),
            })
            .collect(),
        todos: todos
            .into_iter()
            .map(|todo| PanelRow {
                title: todo.title,
                status: format!("{:?}", todo.status),
            })
            .collect(),
        goals: goals
            .into_iter()
            .map(|goal| PanelRow {
                title: goal.title,
                status: format!("{:?}", goal.status),
            })
            .collect(),
    }
}

/// 画到可见 transcript 底部（全空 = 一行不占）；其下是 T3 的问题面板
/// （实占行）与 footer（恒 1 行）——本面板贴这两层顶沿，不盖状态条/
/// 答题卡（route §3 T3、T4 契约同屏共存）。
///
/// 预算夹紧：面板最多吃可见 transcript 高度的一半，且恒给 transcript 留
/// ≥1 行——44×20 下三节全满也不许把聊天区整个挤没。
pub fn render(frame: &mut Frame, app: &App, snapshot: &PanelSnapshot) {
    if snapshot.is_empty() {
        return;
    }
    let (transcript, dock) = layout::split(frame.area(), app.queued().is_some());
    // T3 三明治让行（与 `ui::draw` 同一套算术）：footer 恒 1 行（dock 压底
    // 时才有）+ 问题面板实占行——可见 transcript 底 = 原始底减去这两层。
    let reserved = u16::from(dock.y > 0) + app.questions().rows();
    let bottom = (transcript.y + transcript.height).saturating_sub(reserved);
    let avail = bottom.saturating_sub(transcript.y);
    if avail == 0 {
        return;
    }
    let lines = snapshot_lines(snapshot);
    let budget = (avail as usize / 2)
        .max(1)
        .min(lines.len())
        .min(avail as usize);
    if budget == 0 {
        return;
    }
    let area = Rect {
        x: transcript.x,
        y: bottom - budget as u16,
        width: transcript.width,
        height: budget as u16,
    };
    let shown: Vec<Line> = lines.into_iter().take(budget).collect();
    frame.render_widget(Paragraph::new(shown), area);
}

/// 三节行序列（空节不出场；节头在前、行按 `MAX_ROWS_PER_SECTION` 截尾）。
fn snapshot_lines(snapshot: &PanelSnapshot) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    out.extend(section_lines("任务", &snapshot.tasks));
    out.extend(section_lines("待办", &snapshot.todos));
    out.extend(section_lines("目标", &snapshot.goals));
    out
}

/// 一节：节头（标题 + 条数）+ 至多 [`MAX_ROWS_PER_SECTION`] 行。
fn section_lines(title: &str, rows: &[PanelRow]) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return Vec::new();
    }
    let mut out = vec![Line::from(Span::styled(
        format!("{title} ({})", rows.len()),
        theme::brand_bold(),
    ))];
    for row in rows.iter().take(MAX_ROWS_PER_SECTION) {
        out.push(Line::from(vec![
            Span::styled(format!(" {} ", row.status), status_style(&row.status)),
            Span::styled(row.title.clone(), theme::dim()),
        ]));
    }
    out
}

/// 状态标签配色：终局成功 → dim；失败/取消/放弃 → danger；进行中/开放 → brand。
fn status_style(status: &str) -> Style {
    match status {
        "Done" | "Achieved" => theme::dim(),
        "Failed" | "Cancelled" | "Abandoned" => theme::danger(),
        _ => theme::brand(),
    }
}
