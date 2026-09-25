//! ui/questions — 问题面板（route §3 T3）。
//!
//! [`QuestionPanel`] 消费 `WebDaemon::pending_questions()` 快照（刷新在
//! app.rs 事件循环：`user.question` 推送 + 兜底轮询）；回答走
//! `WebDaemon::answer_question(id, choice)`——本模块只产出
//! [`AnswerRequest`]，RPC 归事件循环。
//!
//! 边界（route §3）：pending 清空 → 面板消失；数字键越界/面板空 → 忽略
//! （不进 composer）；同 id 重复到达 → 以最新 pending 为准（幂等覆盖）。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use web_client::QuestionItem;

use crate::theme;

use super::transcript::sanitize;

/// 一次数字键回答的出站请求：原问题 `question_id` + `choice`（选项下标，
/// 0-based，与 daemon `QuestionAnswer::Select { index }` 同坐标——丢 id 或
/// 发错 index 都会被 `user.answer` 打回，route §4 该测试钉的 bug）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerRequest {
    pub question_id: String,
    pub choice: usize,
}

/// 待决问题面板：渲染队首问题（编号选项 + 描述 + 批次进度），dock 上方。
#[derive(Debug, Default)]
pub struct QuestionPanel {
    /// 当前待答（快照顺序；答完即摘掉，清空即面板消失）。
    pending: Vec<QuestionItem>,
}

impl QuestionPanel {
    /// 空面板（无 pending → `rows()` 为 0、不渲染）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 快照整体替换（`pending_questions()` 是服务端真相）：同一 id 多条
    /// 取最新一条（幂等覆盖），不在新快照里的问题即从面板消失。
    pub fn set_pending(&mut self, items: Vec<QuestionItem>) {
        let mut merged: Vec<QuestionItem> = Vec::with_capacity(items.len());
        for item in items {
            match merged.iter_mut().find(|q| q.id == item.id) {
                Some(slot) => *slot = item,
                None => merged.push(item),
            }
        }
        self.pending = merged;
    }

    /// 面板是否无题（无题 → 面板不占行）。
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// 队首问题（批次进度 = 它在 pending 里的位次）。
    pub fn current(&self) -> Option<&QuestionItem> {
        self.pending.first()
    }

    /// 面板行数：1 行题干 + N 行选项 + 1 行批次进度；无题 = 0。
    pub fn rows(&self) -> u16 {
        self.current().map_or(0, |q| (q.options.len() + 2) as u16)
    }

    /// 数字键 → 出站回答；面板空 / 非数字 / 越界 → `None`（调用方按
    /// route §3 边界把该键吞掉：忽略，不落进 composer）。
    pub fn press_digit(&mut self, digit: char) -> Option<AnswerRequest> {
        let key = digit.to_digit(10)? as usize;
        let choice = key.checked_sub(1)?;
        let question = self.pending.first()?;
        (choice < question.options.len()).then(|| AnswerRequest {
            question_id: question.id.clone(),
            choice,
        })
    }

    /// 回答已交给发送路径：从 pending 摘掉（答完 → 面板消失）。若 daemon
    /// 打回（已答/不存在），随后的快照刷新会把仍 pending 的问题带回来。
    pub fn forget(&mut self, question_id: &str) {
        self.pending.retain(|q| q.id != question_id);
    }

    /// 渲染到 `area`（行数 = [`Self::rows`]；超宽文本由 Paragraph 按列裁）。
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let Some(question) = self.current() else {
            return;
        };
        let mut lines = Vec::with_capacity(question.options.len() + 2);
        lines.push(Line::styled(
            format!("? {}", flat(&question.summary)),
            theme::brand_bold(),
        ));
        for (i, option) in question.options.iter().enumerate() {
            let mut text = format!("{}) {}", i + 1, flat(&option.label));
            if let Some(desc) = &option.description {
                text.push_str(" — ");
                text.push_str(&flat(desc));
            }
            lines.push(Line::styled(text, theme::base()));
        }
        lines.push(Line::styled(
            format!("question {}/{}", 1, self.pending.len()),
            theme::dim(),
        ));
        frame.render_widget(Paragraph::new(lines), area);
    }
}

/// 单行化：滤控制符 + 去换行（面板行数按「一项一行」数，题干里的换行
/// 不许把行数撑出 [`QuestionPanel::rows`] 计的配额）。
fn flat(text: &str) -> String {
    sanitize(text).replace('\n', " ")
}
