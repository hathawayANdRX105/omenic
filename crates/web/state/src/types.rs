//! UI 状态 DTO 词汇表（omenic-web-state::types）。
//!
//! 页面/组件消费的会话、消息、任务、统计等形状，由
//! `ui_state::apply_event` 从 `AgentEvent` 流转译填充（C5.1/G4），
//! 或由 `convert` 从 daemon 存储行映射（C5.2a）。工作区页已无 mock
//! 数据源（G5）：连不上 daemon 时一律空态（见 `StatusLine::empty`）。

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
    /// 谱系边：本会话从哪个父会话 fork 而来（5.3/5.5 分组）。`None` = 根
    /// 会话。serde-optional：前端残留的旧缓存（缺该 key）反序列化成
    /// `None` 而不是报错，保证字段上线时不需要清缓存。
    #[serde(default)]
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SessionStatus {
    Active,
    Idle,
    Archived,
    /// 有开始无结束的半开 run（崩溃/中断孤儿）：刷新后由 `run.list` 的
    /// run 记录组装出来（见 `convert::infer_session_status`）。侧栏状态点
    /// 与 Archived 同走 danger 色。
    Aborted,
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

/// 模型上下文上限的中性默认值（原 mock fixture 也是 128k，保持一致）。
/// 真实 per-model 上限还没有数据源（adaptor 侧未暴露），暂用该常量。
pub const DEFAULT_CONTEXT_MAX: u64 = 128_000;

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
    /// 当前 run 的开始时刻（epoch ms）。`None` = 无在飞 run。
    /// run 开始时由页面 [`StatusLine::start_run`] 落定，结束时清空。
    #[serde(default)]
    pub run_started_at_ms: Option<u64>,
    /// 最近一次已结束 run 的总耗时（ms）。0 = 还没跑过 run。
    /// dsh 对位实现：`assistant-timing.ts` 的 turn 计时。
    #[serde(default)]
    pub elapsed_ms: u64,
}

impl Default for StatusLine {
    /// 空态初值：无 mock、无编造。model/cwd 等由调用方用真实值覆盖；
    /// token/cost/计时一律归零（真实值由 TurnEnd 结算写入）。
    fn default() -> Self {
        Self::empty()
    }
}

impl StatusLine {
    /// 中性零值状态行：所有可计量字段归零，字符串字段留空（没有真实来源
    /// 的字段不编造内容）。`context_max` 取 [`DEFAULT_CONTEXT_MAX`]，
    /// 避免 `context_pct` 计算除零。
    pub fn empty() -> Self {
        StatusLine {
            model: String::new(),
            thinking: "off".into(),
            cwd: String::new(),
            git_branch: String::new(),
            tokens_in: 0,
            tokens_out: 0,
            cost_usd: 0.0,
            context_pct: 0.0,
            context_max: DEFAULT_CONTEXT_MAX,
            run_started_at_ms: None,
            elapsed_ms: 0,
        }
    }

    /// run 开始：记开始时刻并清掉上一轮的总耗时。
    pub fn start_run(&mut self, now_ms: u64) {
        self.run_started_at_ms = Some(now_ms);
        self.elapsed_ms = 0;
    }

    /// run 结束：把开始时刻结算成总耗时。无在飞 run（重复结算 / 中断后
    /// 又收到 TurnEnd）时保持上一次结算结果不变，避免把已算好的耗时清零。
    pub fn finish_run(&mut self, now_ms: u64) {
        if let Some(start) = self.run_started_at_ms.take() {
            self.elapsed_ms = now_ms.saturating_sub(start);
        }
    }

    /// 状态行的耗时文案。在飞 run 用「当前时刻 - 开始时刻」实时算（随流式
    /// 事件重渲染自然刷新，不额外挂每秒定时器）；已结束 run 用结算好的
    /// 总耗时。都没有则返回空串（状态行不显示耗时段）。
    pub fn elapsed_label_at(&self, now_ms: u64) -> String {
        let ms = match self.run_started_at_ms {
            Some(start) => now_ms.saturating_sub(start),
            None if self.elapsed_ms > 0 => self.elapsed_ms,
            None => return String::new(),
        };
        format_duration_ms(ms)
    }

    /// [`Self::elapsed_label_at`] 取系统当前时刻的便捷版本（渲染侧用）。
    pub fn elapsed_label(&self) -> String {
        self.elapsed_label_at(now_epoch_ms())
    }
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

/// 任务卡的中性优先级：run 记录没有优先级概念，统一 P2（渲染成弱化
/// chip）。真正的任务优先级属于 C8 任务子系统范围，此处不编造。
pub const RUN_TASK_PRIORITY: u8 = 2;

impl TaskItem {
    /// daemon run ledger 的一条 run 记录 → 任务卡（G5：TaskPanel 的真实
    /// 数据源）。「任务系统」视图本身属于 C8（暂缓），这里只把已有的真实
    /// run 记录投影成任务卡形状，不新造任务子系统。
    ///
    /// 字段来源：
    /// - `id` = `run_id`（真实）；
    /// - `title` = run 起始时刻的相对时间（真实，`started_at_ms`）；
    /// - `status` = 由 `finished_at_ms` / `status` 映射（真实，见
    ///   [`run_task_status`]）；
    /// - `kind` 固定 `"run"`（真实：这条记录就是一次 agent run）；
    /// - `priority` = [`RUN_TASK_PRIORITY`]（无真实来源，中性值）；
    /// - `description` = 耗时 / 进行中 + 结束状态（真实）；
    /// - `acceptance` 留空——run 记录没有验收标准这一概念，不编造。
    pub fn from_run(
        run_id: &str,
        started_at_ms: i64,
        finished_at_ms: Option<i64>,
        status: Option<&str>,
    ) -> TaskItem {
        let started = started_at_ms.max(0) as u64;
        let description = match finished_at_ms {
            Some(fin) => {
                let dur = format_duration_ms((fin.max(0) as u64).saturating_sub(started));
                match status {
                    Some(s) if !s.is_empty() => format!("耗时 {dur} · {s}"),
                    _ => format!("耗时 {dur}"),
                }
            }
            None => "进行中".to_string(),
        };
        TaskItem {
            id: run_id.to_string(),
            title: format!("运行 · {}", format_relative_time(started)),
            status: run_task_status(finished_at_ms, status).to_string(),
            kind: "run".into(),
            priority: RUN_TASK_PRIORITY,
            description,
            acceptance: String::new(),
        }
    }

    /// CLI 任务存储（`tasks.jsonl`，由 `oi task add/done/...` 写入）的一条
    /// 任务 → 任务卡（任务看板的持久编排数据源，与 [`Self::from_run`] 的
    /// 瞬时执行记录并列）。
    ///
    /// 字段来源（任务存储本身就有这些列，全部真实，没有需要编造的字段）：
    /// - `id` = `task.id`（CLI 用它当 slug）；
    /// - `title` = `task.title`；
    /// - `status` = 由 `task.status` 映射（真实，见
    ///   [`task_status_to_panel`]）；
    /// - `kind` = `task.kind` 的 snake_case 串（[`TaskKind`] 本身 serde 即
    ///   snake_case，这里手写 match 直观可穷尽，避免 `to_value` 的 `Result`
    ///   噪音）；
    /// - `priority` = `task.priority`（真实 u8，P0/P1/其余三档照原样渲染）；
    /// - `description` = `task.description`（可空，TaskPanel 空串不渲染）；
    /// - `acceptance` = `task.acceptance`（可空）。
    pub fn from_task(task: &task::Task) -> TaskItem {
        TaskItem {
            id: task.id.clone(),
            title: task.title.clone(),
            status: task_status_to_panel(&task.status).to_string(),
            kind: task_kind_snake(&task.kind).to_string(),
            priority: task.priority,
            description: task.description.clone(),
            acceptance: task.acceptance.clone(),
        }
    }
}
/// run 记录 → 任务卡状态串（TaskPanel 的过滤/chip 词汇表）。
///
/// daemon 侧只写三种终态（`dispatch.rs`：`"ok"` / `"failed"` /
/// `"spawn_failed"`），未结束的 run 没有 `finished_at_ms`：
/// - 未结束 → `"in_progress"`；
/// - `"ok"` → `"done"`；
/// - 其余终态（失败 / 拉起失败 / 未知串）→ `"blocked"`（danger chip）。
pub fn run_task_status(finished_at_ms: Option<i64>, status: Option<&str>) -> &'static str {
    match (finished_at_ms, status) {
        (None, _) => "in_progress",
        (Some(_), Some("ok")) => "done",
        (Some(_), _) => "blocked",
    }
}

/// 任务存储 → 任务卡状态串（TaskPanel 的过滤/chip 词汇表）。
///
/// [`task::TaskStatus`] 四态映到面板仅有的四个词：
/// - `Open` → `"open"`；
/// - `InProgress` → `"in_progress"`；
/// - `Done` → `"done"`；
/// - `Failed` → `"blocked"`（danger chip：失败态语义上就是需要人工介入的
///   阻塞，和 `run_task_status` 把失败 run 归 blocked 同一理由）。
pub fn task_status_to_panel(status: &task::TaskStatus) -> &'static str {
    match status {
        task::TaskStatus::Open => "open",
        task::TaskStatus::InProgress => "in_progress",
        task::TaskStatus::Done => "done",
        task::TaskStatus::Failed => "blocked",
    }
}

/// `TaskKind` → snake_case 串（与 `TaskKind` 的 serde 序列化一一对应）。
/// 手写 match 而非 `serde_json::to_value`：取一个字符串不必处理 `Result`，
/// 且新增变体时编译器强制补全分支。
fn task_kind_snake(kind: &task::TaskKind) -> &'static str {
    match kind {
        task::TaskKind::Milestone => "milestone",
        task::TaskKind::Feature => "feature",
        task::TaskKind::Bug => "bug",
        task::TaskKind::Task => "task",
        task::TaskKind::Chore => "chore",
        task::TaskKind::Spike => "spike",
        task::TaskKind::Decision => "decision",
        task::TaskKind::Unknown => "unknown",
    }
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

/// 当前 Unix 时刻（epoch ms）。计时/相对时间共用一处取时。
pub fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// 时长（ms）→ 紧凑文案："820ms" / "4.2s" / "1m12s" / "1h03m"。
/// 状态行与任务卡描述共用，保证同一份耗时在两处显示一致。
pub fn format_duration_ms(ms: u64) -> String {
    if ms < 1_000 {
        return format!("{ms}ms");
    }
    let secs = ms / 1_000;
    if secs < 60 {
        return format!("{:.1}s", ms as f64 / 1_000.0);
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{}m{:02}s", mins, secs % 60);
    }
    format!("{}h{:02}m", mins / 60, mins % 60)
}

/// 把毫秒时间戳转成相对时间："刚刚"/"X 分钟前"/"X 小时前"/"昨天 HH:MM"
pub fn format_relative_time(epoch_ms: u64) -> String {
    if epoch_ms == 0 {
        return "刚刚".into();
    }
    let now = now_epoch_ms();
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
