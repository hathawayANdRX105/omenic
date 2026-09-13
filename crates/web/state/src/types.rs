//! UI 状态 DTO 词汇表（omenic-web-state::types）。
//!
//! 页面/组件消费的会话、消息、任务、统计等形状。后续由
//! `ui_state::apply_event` 从 `AgentEvent` 流转译填充（C5.1/G4），
//! 真实数据接线前由 `omenic-web-mock` 提供假数据。

use serde::{Deserialize, Serialize};

// ── Chat ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub title: String,
    pub kind: String, // "bash", "edit", "read", "grep", ...
    pub summary: String,
    pub detail: String,
    pub status: String, // "success", "running", "error"
}

/// 消息体内的有序片段：文本段或一次工具调用。
/// 用于按真实发生顺序渲染 agent 的工作过程（文本与工具调用交叉）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MessagePart {
    Text(String),
    Tool(ToolCall),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    /// 真实发生顺序的有序片段；为空时回退到 content + tool_calls 渲染。
    #[serde(default)]
    pub parts: Vec<MessagePart>,
    pub timestamp: String,
    /// Unix epoch milliseconds — 真实落库时间戳,用于渲染相对时间
    #[serde(default)]
    pub ts_epoch_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub last_active: String,
    pub model: String,
    pub status: SessionStatus,
    /// Unix epoch milliseconds for last activity (for real relative-time display)
    #[serde(default)]
    pub last_active_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SessionStatus {
    Active,
    Idle,
    Archived,
}

/// 侧边栏的项目/工作区条目（真实接线时来自 `git worktree list`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceSpace {
    pub id: String,
    pub name: String,
    pub path: String,
    pub branch: String,
    pub is_active: bool,
}

// ── Status line ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusLine {
    pub model: String,
    pub thinking: String,
    pub cwd: String,
    pub git_branch: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
    pub context_pct: f64,
    pub context_max: u64,
}

// ── Tasks (mirrors crates/task) ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskItem {
    pub id: String,
    pub title: String,
    pub status: String,
    pub kind: String,
    pub priority: u8,
    pub description: String,
    pub acceptance: String,
}

// ── Stats ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KpiCard {
    pub label: String,
    pub value: String,
    pub delta: String,
    pub delta_positive: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubMetric {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentTokenBar {
    pub agent: String,
    pub tokens: String,
    pub pct: f64,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThroughputPoint {
    pub time: String,
    pub requests: f64,
    pub tokens: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeedItem {
    pub model: String,
    pub provider: String,
    pub time_ago: String,
    pub duration: String,
    pub cost: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatsData {
    pub kpis: Vec<KpiCard>,
    pub sub_metrics: Vec<SubMetric>,
    pub agent_bars: Vec<AgentTokenBar>,
    pub throughput: Vec<ThroughputPoint>,
    pub feed: Vec<FeedItem>,
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// 把毫秒时间戳转成相对时间："刚刚"/"X 分钟前"/"X 小时前"/"昨天 HH:MM"
pub fn format_relative_time(epoch_ms: u64) -> String {
    if epoch_ms == 0 {
        return "刚刚".into();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let age_ms = now.saturating_sub(epoch_ms);
    let secs = age_ms / 1000;
    if secs < 60 {
        return "刚刚".into();
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins} 分钟前");
    }
    let hrs = mins / 60;
    if hrs < 24 {
        return format!("{hrs} 小时前");
    }
    let days = hrs / 24;
    match days {
        1 => "昨天".into(),
        _ => format!("{days} 天前"),
    }
}
