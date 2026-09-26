//! app.rs — enhanced 外壳：会话状态 + 按键路由 + 全屏事件循环（route §3 T2）。
//!
//! 两层（route §2 纯函数管线）：[`App`] 是无 IO 纯状态（按键进去、
//! [`KeyAction`] / 出站队列出来，测试不碰真终端）；[`run_enhanced`] 拿它
//! 接真终端——[`TermGuard`] 进出、ratatui 事件循环、worker prompt 独立线程。
//! 调用方给的 `rx` 是会话级订阅，泵收线 = 订阅断线 = daemon 断线 →
//! 退出码 3 单行错误（route §3 边界）。

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};
use web_client::ClientError;
use web_client::daemon::WebDaemon;
use web_client::{QuestionAnswer, QuestionItem};
use web_state::types::{ChatMessage, MessagePart, format_duration_ms, now_epoch_ms};
use web_state::ui_state::{AgentEvent, UiState};

use crate::scroll::ScrollModel;
use crate::search::{self, SearchState};
use crate::slash::{self, Action, Intent};
use crate::termguard::{CrosstermOps, TermGuard};
use crate::ui::footer;
use crate::ui::questions::{AnswerRequest, QuestionPanel};
use crate::{
    TuiError, TuiOptions, client_error, persist_assistant, push_user_message, resolve_session,
};

/// 事件循环的按键轮询间隔（draw 在每次轮询前，事件来了即刻重画）。
const POLL: Duration = Duration::from_millis(50);

/// T3：pending 快照兜底轮询间隔（`user.question` 推送之外的保底刷新）。
const PENDING_SYNC: Duration = Duration::from_secs(1);

/// T3：`stats.summary` 刷新间隔（footer 空闲段的 run 状态）。
const STATS_SYNC: Duration = Duration::from_secs(2);

/// T3：`stats.summary` 的统计窗口（footer 只用其中的半开 run 计数）。
const STATS_RANGE: &str = "24h";

/// T9：`/theme` 回显文案（主题能力现状：TUI 只有内置单套命名色，见
/// `theme.rs` D11；`/theme` 切换是 T17 的活——编不到就不编，明说没有）。
const THEME_LINE: &str = "theme: default — palette switching not available yet";

/// T12：copy 的短反馈文案（dock 活动行覆写；随下一次按键或状态事件退场，
/// 见 [`App::handle_key`] 开头的清理点）。OSC52 是否被终端采纳不可证，
/// 这行字只陈述「本方已写入 stdout」这一已证明事实（route §3 T12 注记①）。
const COPIED_STATUS: &str = "copied";

// --- T10 运行中排队常数（route §3 T10；dh-rs `PromptQueue` 口径，任务书
// §2 实录：`input_memory.rs:11` 的 MAX_QUEUE_ITEMS、`composer.rs:10` 的
// 64KiB——字节上限按**队列总量**记账，不是单条上限） ---

/// 队列条数上限：满员再入队即拒收（dh-rs `enqueue_from` 的 `QueueFull`
/// 同构），状态行明说、不静默丢（route §3）。
const MAX_QUEUE_ITEMS: usize = 8;

/// 队列字节**总量**上限：累计超过 64KiB 拒收（dh-rs `next_total >
/// MAX_QUEUE_BYTES` 同款判据——入队前先算总量，不等溢出）。
const MAX_QUEUE_BYTES: usize = 64 * 1024;

// --- T7 鼠标滚轮归一化常数（route §3 T7；出处 grok-build refs
// `xai-grok-pager-render/src/input/mouse.rs:63-76`，四常数与量纲由
// `tests/wheel_scroll.rs::wheel_constants_snapshot` 钉死防漂移） ---

/// 每个滚轮 notch 入队的行数（grok `DEFAULT_WHEEL_LINES_PER_TICK`：
/// 终端只报方向不报量级，一个物理 tick 恒定滚 WHEEL_LINES_PER_TICK 行）。
pub const WHEEL_LINES_PER_TICK: i32 = 3;

/// 冲刷节流下限（grok `REDRAW_CADENCE_MS`，≈60fps）：相邻两次缓出冲刷的
/// 间隔不得小于该值；16ms 内到达的 drain tick 直接跳过。
pub const REDRAW_CADENCE_MS: u64 = 16;

/// 流间隔（grok `STREAM_GAP_MS`）：与上一个 notch 相隔超过该值即视为
/// 上一手势（流）已收尾、本 notch 开新流——本版单队列模型下新流的折算
/// 口径见 [`App::wheel_tick`]（换向清残留、同向余量继续缓出），
/// trackpad 档（流级状态）接入时该常数直接接管流边界判定。
pub const STREAM_GAP_MS: u64 = 80;

/// 滚轮 tick 判窗（grok `DEFAULT_WHEEL_TICK_DETECT_MAX_MS`）：连续 notch
/// 间隔 ≤ 该值属同一物理 tick 批次（终端一批多报 / 手势抖动），归并进同一
/// 队列；出判窗的反向 notch = 换向新流，先清残留反向队列。
pub const WHEEL_TICK_DETECT_MAX_MS: u64 = 12;

/// 滚轮队列上限（防雪崩：高分 wheel / 触控板事件密度再高，待出行数也
/// clamp 在 ±该值内；抄 jcode `MOUSE_SCROLL_MAX_QUEUE` 的抄写位，本版取
/// ±128——超限的 notch 直接丢弃，不让待出行数无限堆积）。
pub const WHEEL_QUEUE_MAX: i32 = 128;

/// 一次按键路由的结论（提交走 [`App`] 内部出站队列，不出现在这里）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    /// 无事发生（含「请求退出确认」的第一下 Esc/Ctrl+C）。
    None,
    /// 运行中按 Esc/Ctrl+C：中止当前轮（发 `worker.abort`，不退出）。
    Abort,
    /// 空闲下二次 Esc/Ctrl+C 确认，或 Ctrl+D 空 composer：退出。
    Quit,
}

/// 出站 prompt 的路由三元组（route §3 T4 红线）：事件循环按这三元组
/// 落库（`session_id`）、起 run 订阅（`run_id`）、发 `worker_prompt_run`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptDelivery {
    /// 目标会话（切会话后必须是新选中的那个）。
    pub session_id: String,
    /// 本次运行的 run_id（每次出站新生成，旧值不复用）。
    pub run_id: String,
    /// prompt 正文。
    pub text: String,
}

/// enhanced 外壳的会话状态（无 IO，可被测试直接驱动）。
#[derive(Default)]
pub struct App {
    /// composer 当前行。
    input: String,
    /// ↑ 召回历史时暂存的未提交草稿（↓ 越过最新一条后恢复）。
    draft: String,
    /// 已提交 prompt（本地输入历史，oldest first）。
    history: Vec<String>,
    /// 历史游标：`None` = 未在浏览；`Some(0)` = 最新一条。
    history_pos: Option<usize>,
    /// 出站队列（T10 FIFO：空闲时头部即刻发，运行中整队等本轮 `TurnEnd`
    /// 逐条消费；上限 [`MAX_QUEUE_ITEMS`] 条 / [`MAX_QUEUE_BYTES`] 总字节）。
    outgoing: VecDeque<String>,
    /// T10：队列字节总量记账（入队加、召回/出队同步减；`queued: n` 计数
    /// 走 [`Self::queued_count`]，字节账没有渲染面——只喂上限判定）。
    queue_bytes: usize,
    /// 本轮是否在跑（submit 置位、`TurnEnd` 清零）。
    running: bool,
    /// 退出确认未决（空闲第一下 Esc 置位，第二下才退）。
    confirm_quit: bool,
    /// 活动行覆写（错误 / aborting 文案）；空 = 按 running 派生。
    status: String,
    /// transcript 投影（T1 同一 `UiState::apply` 形状，会话内累积）。
    ui: UiState,
    /// T3：工具卡展开态（一个键全部展开/折叠）。
    tools_expanded: bool,
    /// T3：问题面板（dock 上方渲染，route §3）。
    questions: QuestionPanel,
    /// T3：数字键产出的待发送回答（事件循环取出走 `answer_question`）。
    answers: VecDeque<AnswerRequest>,
    /// T3：footer 的 model 段（事件循环注入运行时配置；测试可显式注入）。
    model: String,
    /// T3：`stats.summary` 的半开 run 计数（footer 空闲段 run 状态）。
    in_flight: u64,
    /// T3：本轮开始时刻（footer 耗时段；无 run = `None`）。
    run_started: Option<Instant>,
    /// 当前会话 id（prompt 路由的 session 字段，route §3 T4 红线）。
    session_id: String,
    /// 当前在飞 run 的 id（`None` = 无；take_prompt 置位、turn 收尾清零）。
    current_run: Option<String>,
    /// run_id 单调时钟（毫秒；同 ms 连发两个 prompt 也保证 run_id 不重复）。
    run_clock: u64,
    /// 事件准入是否已切到 run 作用域（首次切会话后为 `true`：会话级
    /// `rx` 只做断线探测，视图事件按 run_id 从 [`Self::apply_run_event`] 进）。
    run_scoped: bool,
    /// 历史滚动：从底部向上滚过的消息条数（0 = 钉底；边界见
    /// [`Self::scroll_by`]）。
    scroll: usize,
    /// T6：transcript 视口滚动（渲染行粒度：PgUp/PgDn/Ctrl+U/End + 脱钩
    /// 跟尾；与上面 `scroll` 的消息条数滚动是两套状态，见 `scroll` 模块）。
    viewport: ScrollModel,
    /// T7：滚轮待出行数（正 = 向下行待走；每 notch ±[`WHEEL_LINES_PER_TICK`]，
    /// 事件循环按 [`WHEEL_QUEUE_MAX`] clamp 后入队、按 3/2/1 缓出冲刷）。
    wheel_queue: i32,
    /// T7：上一个 notch 的 (时刻, 方向)——[`WHEEL_TICK_DETECT_MAX_MS`] 判窗
    /// 与流间隔 gap 的基准；`None` = 尚无滚轮输入。
    wheel_last: Option<(Instant, i32)>,
    /// T7：上次缓出冲刷时刻（[`REDRAW_CADENCE_MS`] 下限判定；`None` = 尚未
    /// 冲刷过，首刷不等节流）。
    wheel_flushed: Option<Instant>,
    /// T9：斜杠面板抑制态（Escape 关闭 / 第一段 Enter 补全后置位；composer
    /// 再次编辑即复位重新武装，见 [`Self::slash_edited`]）。
    slash_suppressed: bool,
    /// T9：面板高亮游标（编辑回顶；读取时按候选数夹紧，见 [`Self::slash_selected`]）。
    slash_selected: usize,
    /// T9：斜杠命令总闸（enhanced 恒开；inline 档 [`Self::disable_slash`]
    /// 关闸——面板 enhanced 专属，route §3 T9）。
    slash_enabled: bool,
    /// T9：命令意图出站队列（`App` 无 IO：入队不出手，事件循环按
    /// [`Self::take_intent`] 消费，picker/RPC 归它）。
    intents: VecDeque<slash::Intent>,
    /// T11：搜索 overlay 状态（开合/查询/命中/视口快照，route §3 T11；
    /// 键路由短路与 `ui::areas` 让行都读它，见 [`Self::search_open`]）。
    search: SearchState,
    /// T11：overlay 总闸（enhanced 恒开；inline 档 [`Self::disable_search`]
    /// 关闸——overlay 仅 enhanced，回看/搜索/复制归终端，同 `disable_slash`
    /// 口径）。
    search_enabled: bool,
    /// T12：消息操作总闸（enhanced 恒开；inline 档
    /// [`Self::disable_message_ops`] 关闸——inline 没有 transcript 重绘，
    /// 截断后视图无从改写，回看/复制照 T8 归终端，同 `disable_slash` /
    /// `disable_search` 口径）。
    message_ops_enabled: bool,
    /// T12：聚焦的消息下标（`ui.messages` 的下标；`None` = 无焦点）。
    /// BackTab 前向环绕推进、Esc 清焦，渲染侧按它画焦点行标记。
    focused: Option<usize>,
    /// T12：改写挂起（聚焦 user 按 `e` 后的 `(消息下标, ledger seq)`）——
    /// 回填不改历史，**提交才消费**：出站时截断 `seq >=` 该条并裁剪视图。
    /// composer 被清空 = 放弃改写（见 [`Self::retire_stale_edit`]）。
    edit_pending: Option<(usize, i64)>,
    /// T12：下一次出站前要执行的 `session.truncate(from_seq)`——retry 在
    /// 入队时置位、edit 在提交时置位，事件循环 [`Self::take_truncate`]
    /// 消费（App 无 IO，RPC 归事件循环，同 T9 intent / T11 取数的分工）。
    truncate_pending: Option<i64>,
    /// T12：待写出的 OSC52 剪贴板正文（`c` 入队，事件循环取出写 stdout）。
    copies: VecDeque<String>,
    /// T13：rewind 确认态 `(from_seq, n)` = 已校验的回退点与待丢弃轮数——
    /// **模态**：[`Self::handle_key`] 在它存在时拦截全部键（y/Enter 确认、
    /// n/Esc/Ctrl+C 取消、其余忽略），状态行显示 `discard n turn(s)? [y/n]`。
    rewind_confirm: Option<(i64, i64)>,
    /// T13：确认后待执行的 `session.rewind(from_seq)` 动作（n 供结果状态行）
    /// ——App 无 IO，事件循环 [`Self::take_rewind`] 消费（同 T12
    /// [`Self::take_truncate`] 的分工）。
    rewind_pending: Option<(i64, i64)>,
    /// T6/T11：最近一次 `ui::sync_viewport` 的 transcript 列宽（命中行
    /// 定位按这个宽度断行；0 = 尚未喂过几何，跳转 no-op）。
    view_width: u16,
}

impl App {
    /// 空会话状态（事件循环与测试的同一入口）。
    pub fn new() -> Self {
        // T9：斜杠总闸默认开、T11：搜索 overlay 总闸默认开（`#[derive(Default)]`
        // 的 bool 落 false，这里显式翻开；inline 档随后 [`Self::disable_slash`] /
        // [`Self::disable_search`] 关闸）。T12：消息操作总闸同一批翻开。
        Self {
            slash_enabled: true,
            search_enabled: true,
            message_ops_enabled: true,
            ..Self::default()
        }
    }

    /// 喂一个字符按键（测试与事件循环共用的打字辅助）。
    pub fn type_char(&mut self, c: char) -> KeyAction {
        self.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    /// composer 当前行。
    pub fn input(&self) -> &str {
        &self.input
    }

    /// T9：斜杠面板是否可见（总闸开 + 整行以 `/` 开头 + 未被抑制——非行首
    /// `/` 永不触发，route §3 T9）。面板渲染与按键短路都读这一个判据。
    /// T11：搜索 overlay 打开时面板让位（overlay 短路了全部按键，面板此刻
    /// 既不可操作也不该叠着渲染——开合两态互斥，composer 原文不受影响）。
    pub fn slash_visible(&self) -> bool {
        self.slash_enabled
            && !self.search_open()
            && !self.slash_suppressed
            && self.input.starts_with('/')
    }

    /// T9：面板候选（注册表按 composer 整行模糊过滤，保持注册表顺序；
    /// 总闸关 = 空）。面板可见时的导航、补全与渲染共用这一份。
    pub fn slash_matches(&self) -> Vec<&'static slash::Command> {
        if !self.slash_enabled {
            return Vec::new();
        }
        slash::filter(&self.input)
    }

    /// T9：高亮候选（按 [`Self::slash_matches`] 夹紧后的游标项；面板隐藏
    /// 或无命中 = `None`）。
    pub fn slash_selected(&self) -> Option<&'static slash::Command> {
        if !self.slash_visible() {
            return None;
        }
        let matches = self.slash_matches();
        matches.get(self.slash_cursor(matches.len())).copied()
    }

    /// T9：面板占行数（0 = 关闭不占行；无命中也占 1 行提示——`ui::areas`
    /// 按这个数从 transcript 让行）。
    pub fn slash_rows(&self) -> u16 {
        if !self.slash_visible() {
            return 0;
        }
        self.slash_matches().len().max(1) as u16
    }

    /// T9：取走一条命令意图（事件循环消费；无 = `None`）。
    pub fn take_intent(&mut self) -> Option<slash::Intent> {
        self.intents.pop_front()
    }

    /// T9：关掉斜杠面板（inline 档专用：面板 enhanced 专属，行首 `/` 在
    /// inline 保持 T8 普通文本语义——不补全、不当命令拦截）。
    pub fn disable_slash(&mut self) {
        self.slash_enabled = false;
    }

    /// T9：面板高亮游标的夹紧位置（候选空 = 0）。
    fn slash_cursor(&self, len: usize) -> usize {
        if len == 0 {
            0
        } else {
            self.slash_selected.min(len - 1)
        }
    }

    // --- T11 转录搜索 overlay（route §3 T11；状态在 [`SearchState`]，
    // RPC 归事件循环，本组是观察缝与开合落点） ---

    /// T11：overlay 是否打开（键路由短路与 `ui::areas` 让行的同一判据）。
    pub fn search_open(&self) -> bool {
        self.search.is_open()
    }

    /// T11：overlay 占行数（0 = 关闭；开 = [`search::OVERLAY_ROWS`]，
    /// `ui::areas` 按这个数从 transcript 让行——渲染与让行同源，
    /// 同 [`Self::slash_rows`] 口径）。
    pub fn search_rows(&self) -> u16 {
        if self.search_open() {
            search::OVERLAY_ROWS
        } else {
            0
        }
    }

    /// T11：只读观察缝（渲染与测试读查询词 / 命中 / 状态文案 / 视口快照）。
    pub fn search_state(&self) -> &SearchState {
        &self.search
    }

    /// T11：事件循环的取数判据——打开 + 查询变更 + 非空词。空查询不发
    /// RPC（存储侧空查询是 `InvalidSearchQuery` 错误；状态行显 `type to
    /// search`，不冒充无命中）。
    pub fn search_needs_fetch(&self) -> bool {
        self.search_open() && self.search.dirty() && !self.search.query().trim().is_empty()
    }

    /// T11：查询词（事件循环取走发 RPC；返回 owned 避免与 `&mut self`
    /// 的后续注入撞借用）。
    pub fn search_query(&self) -> String {
        self.search.query().to_string()
    }

    /// T11：注入 `session.search` 回执——把命中映射到当前 transcript 的
    /// 消息下标（[`search::map_hits`]），并记上触顶标志（`FETCH_LIMIT`
    /// 打满 = 结果可能截断，状态行 `N+ matches` 不装全量）。
    pub fn set_search_hits(&mut self, rows: Vec<ChatMessage>) {
        let capped = rows.len() as u32 >= search::FETCH_LIMIT;
        let hits = search::map_hits(&self.ui.messages, &rows);
        self.search.set_hits(hits, capped);
    }

    /// T11：取数失败可见（真值安全：状态行报错，不静默、不假装无命中）。
    pub fn set_search_error(&mut self, message: impl Into<String>) {
        self.search.set_error(message.into());
    }

    /// T11：inline 档关闸（route §3 T8 设计注记：回看/搜索/复制全归终端，
    /// overlay 仅 enhanced；同 [`Self::disable_slash`] 口径——关闸后 Ctrl+R
    /// 与 `/search` 都不产生 overlay，键位行为与合入前一致）。
    pub fn disable_search(&mut self) {
        self.search_enabled = false;
    }

    /// T11：开 overlay（Ctrl+R 与 `/search` 的**同一落点**，双入口等效由
    /// `tests/search_overlay.rs` 钉）：快照当前视口供 ESC 还原（bug 本体：
    /// 搜索后滚动位置丢失）、复位查询与命中、撤掉未决退出确认（dock 提示
    /// 承诺 other key cancels，开 overlay 也算 other key）。总闸关 /
    /// 已打开 = no-op。
    pub fn open_search(&mut self) {
        if !self.search_enabled || self.search.is_open() {
            return;
        }
        self.confirm_quit = false;
        self.search.open(self.viewport);
    }

    /// T11：关 overlay（ESC 落点）：交还打开前的视口快照——`close` 先取
    /// 快照再复位状态，写回视口后下一次 `sync_viewport` 按当时几何 clamp，
    /// offset/follow 原样回来（bug：搜索后滚动位置丢失的修复正本）。
    pub fn close_search(&mut self) {
        if let Some(saved) = self.search.close() {
            *self.viewport_mut() = saved;
        }
    }

    /// T11：视图整体更换（切会话 / 清屏）：命中行号与视口快照全部作废，
    /// overlay 一并收掉——调用方随后 reset 视口，不走 [`Self::close_search`]
    /// 的还原语义（旧台的滚动位置没有意义）。
    pub fn forget_search(&mut self) {
        self.search.forget();
    }

    /// T11：overlay 打开时的整段短路路由（[`Self::handle_key`] 最先分派，
    /// 同 T4 picker 的分流结构）：查询编辑 / 命中导航 / ESC 关闭在这里
    /// 收口，composer、退出确认、翻页键、Ctrl+K、Ctrl+C/D 一律不漏——
    /// 全部返回 [`KeyAction::None`]（不产生退出/中断/入队副作用）。
    fn search_key(&mut self, key: KeyEvent) -> KeyAction {
        match key.code {
            // ESC：关 overlay + 还原视口（本任务 bug 正本）。
            KeyCode::Esc => self.close_search(),
            // 导航：Enter 下一条 / Shift+Enter 上一条（循环边界）。
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.search_step(true);
            }
            KeyCode::Enter => self.search_step(false),
            // 查询编辑：普通字符入词、退格删词（Alt/Ctrl 组合落 `_` 吞掉）。
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.search.edit(|query| query.push(c));
            }
            KeyCode::Backspace => self.search.edit(|query| {
                query.pop();
            }),
            // 其余键（翻页/历史/提交/退出/切换/工具卡…）一律吞掉：overlay
            // 期间没有 composer 语义，按键提示在 overlay 状态行自述。
            _ => {}
        }
        KeyAction::None
    }

    /// T11：跳上/下一条命中并把视口定位到该行（跳转即滚动定位，复用 T6
    /// 视口 API——[`SearchState::step`] 算目标，[`Self::jump_to_hit`] 落位；
    /// 无命中 = 不动视口）。
    fn search_step(&mut self, back: bool) {
        if let Some(index) = self.search.step(back) {
            self.jump_to_hit(index);
        }
    }

    /// T11：把视口挪到命中行——行坐标 = [`crate::ui::search_overlay::line_offset`]
    /// （命中消息之前的渲染行数，transcript 断行核心只读复用），落位走
    /// [`ScrollModel::scroll_lines`]（视口首行差值；clamp / 脱钩语义全在
    /// 既有模型里，不发明新边界）。命中不在视图（`msg = None`）或几何未
    /// 喂过（`view_width = 0`）= 跳转 no-op，不编造行号。
    fn jump_to_hit(&mut self, index: usize) {
        let Some(hit) = self.search.hits().get(index) else {
            return;
        };
        let Some(msg) = hit.msg else {
            return;
        };
        if self.view_width == 0 {
            return;
        }
        let line = crate::ui::search_overlay::line_offset(
            &self.ui.messages,
            self.tools_expanded,
            msg,
            self.view_width,
        );
        let top = {
            let view = &self.viewport;
            view.view_top(view.total(), view.height())
        };
        let delta = (line as i64 - top as i64).clamp(i32::MIN as i64, i32::MAX as i64);
        self.viewport.scroll_lines(delta as i32);
    }

    /// T6/T11：`ui::sync_viewport` 喂本帧几何时同步记录列宽（命中行定位
    /// 的断行坐标；测试走同一入口，不绕开模型自己算宽度）。
    pub(crate) fn set_view_width(&mut self, width: u16) {
        self.view_width = width;
    }

    // --- T12 消息操作 retry / edit / copy（route §3 T12 设计注记五条定案；
    // 操作路由在 [`Self::handle_key`]，本组只出观察缝与状态回填） ---

    /// T12：聚焦消息下标（渲染画焦点行标记、测试读；`None` = 无焦点）。
    pub fn focused_message(&self) -> Option<usize> {
        self.focused
    }

    /// T12：inline 档关闸（route §3 T8 设计注记：回看/搜索/复制归终端——
    /// inline 的 transcript 是写即定稿的 scrollback，截断后视图无从改写，
    /// 聚焦操作在这里与斜杠面板/搜索 overlay 同批熄火，键位与合入前一致）。
    pub fn disable_message_ops(&mut self) {
        self.message_ops_enabled = false;
    }

    /// T12：取走一条待写出的 OSC52 正文（事件循环消费；`App` 无 IO，
    /// 同 [`Self::take_intent`] / [`Self::take_answer`] 的分工）。
    pub fn take_copy(&mut self) -> Option<String> {
        self.copies.pop_front()
    }

    /// T12：取走下一次出站要执行的 `session.truncate(from_seq)`（事件循环
    /// 在落库/订阅/prompt 之前消费——截断必须先于重发文落库，否则重发的
    /// 消息自己也被删）。
    pub fn take_truncate(&mut self) -> Option<i64> {
        self.truncate_pending.take()
    }

    /// T13：确认态 `(from_seq, n)`（渲染/测试读；`None` = 不在确认态）。
    pub fn rewind_confirm(&self) -> Option<(i64, i64)> {
        self.rewind_confirm
    }

    /// T13：取走确认后的回退动作 `(from_seq, n)`（事件循环消费去发 RPC，
    /// App 无 IO——同 [`Self::take_truncate`] 的分工）。
    pub fn take_rewind(&mut self) -> Option<(i64, i64)> {
        self.rewind_pending.take()
    }

    /// T12：出站消息落库回执——把本地投影里刚推的那条 user 消息 id 回填成
    /// ledger 口径 `{session_id}-{seq}`（`message_to_chat` 同一套序号）。
    /// retry/edit 的 seq 只认这个口径：不回填就只认得历史回填的消息，
    /// 本轮刚发的那条反而截不断。
    pub fn note_appended_seq(&mut self, seq: i64) {
        let id = format!("{}-{}", self.session_id, seq);
        if let Some(msg) = self.ui.messages.last_mut()
            && msg.role == "user"
        {
            msg.id = id;
        }
    }

    /// 本轮是否在跑。
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// 退出确认是否未决（dock 提示行消费）。
    pub fn confirm_quit(&self) -> bool {
        self.confirm_quit
    }

    /// 队首待发 prompt（`None` = 队列空；dock 排队行显隐与 inline 状态行
    /// 标记的判据，正文预览不渲染——排队行显示计数，见 [`Self::queued_count`]）。
    pub fn queued(&self) -> Option<&str> {
        self.outgoing.front().map(String::as_str)
    }

    /// T10：队列条数（dock/composer 状态行 `queued: n` 的 n，入队/召回/
    /// 消费三点都从这里读——与上限判定同一数据源 [`Self::outgoing`]）。
    pub fn queued_count(&self) -> usize {
        self.outgoing.len()
    }

    /// 活动行覆写文案（空 = 由 running 派生 running/idle）。
    pub fn status_text(&self) -> &str {
        &self.status
    }

    /// transcript 消息快照。
    pub fn messages(&self) -> &[ChatMessage] {
        &self.ui.messages
    }

    /// footer 的 model 段（事件循环注入运行时配置）。
    pub fn model(&self) -> &str {
        &self.model
    }

    /// 注入 footer 的 model 段（`footer::configured_model`；测试显式给值）。
    pub fn set_model(&mut self, model: impl Into<String>) {
        self.model = model.into();
    }

    /// 记入 `stats.summary` 的半开 run 计数（footer 空闲段数据源）。
    pub fn set_in_flight(&mut self, in_flight: u64) {
        self.in_flight = in_flight;
    }

    /// footer 耗时段：运行中 = 本轮开始至今；无 run = 空串（段被跳过）。
    pub fn elapsed_label(&self) -> String {
        self.run_started.map_or_else(String::new, |start| {
            format_duration_ms(start.elapsed().as_millis() as u64)
        })
    }

    /// footer 空闲段的 run 状态（已核字段：`stats.summary` 的半开 run
    /// 计数——per-context 占用核不到，route §3 不许编百分比）。
    pub fn run_state_label(&self) -> String {
        if self.in_flight > 0 {
            format!("{} in flight", self.in_flight)
        } else {
            "idle".to_string()
        }
    }

    /// 工具卡展开态（一个键全部展开/折叠，route §3）。
    pub fn tools_expanded(&self) -> bool {
        self.tools_expanded
    }

    /// 问题面板（渲染与数字键路由读它）。
    pub fn questions(&self) -> &QuestionPanel {
        &self.questions
    }

    /// 换入最新 `pending_questions()` 快照（服务端真相整体替换；同 id
    /// 幂等覆盖、答完消失的语义在 [`QuestionPanel::set_pending`]）。
    pub fn set_pending_questions(&mut self, items: Vec<QuestionItem>) {
        self.questions.set_pending(items);
    }

    /// 取一条待发送回答，同时把该题从面板摘掉（答完 → 面板消失，route §3）。
    pub fn take_answer(&mut self) -> Option<AnswerRequest> {
        let request = self.answers.pop_front()?;
        self.questions.forget(&request.question_id);
        Some(request)
    }

    /// 覆写活动行文案（错误 / aborting）。
    pub fn set_status(&mut self, status: impl Into<String>) {
        self.status = status.into();
    }

    /// 折一个 worker 事件进 transcript 投影。
    pub fn apply_event(&mut self, ev: &AgentEvent) {
        self.ui.apply(ev);
    }

    /// 本轮收尾：回空闲、清活动覆写、耗时起点与在飞 run（晚到的旧 run
    /// 帧随之被 [`Self::apply_run_event`] 拒收）；队首 prompt 随即具备
    /// 出站资格。
    pub fn note_turn_end(&mut self) {
        self.running = false;
        // T13：确认态的提示行（`discard n turn(s)? [y/n]`）不许被迟到的
        // TurnEnd 清掉——模态问题面板还在等 y/n，清了就没提示可读了。
        // 其余情形照旧清活动覆写（合入前逐字一致）。
        if self.rewind_confirm.is_none() {
            self.status.clear();
        }
        self.run_started = None;
        self.current_run = None;
    }

    /// 出站队列头（文本视图，空闲才出队）。语义同
    /// [`Self::take_prompt`] 的出站副作用（置 running、起耗时计时、折
    /// user 消息进 transcript 投影），只要文本不要路由字段。
    pub fn next_to_send(&mut self) -> Option<String> {
        self.take_prompt().map(|delivery| delivery.text)
    }

    /// 出站 prompt 的路由三元组（route §3 T4 红线的落点）：事件循环拿这
    /// 三元组去落库 / 起 run 订阅 / 发 prompt——session_id 取自切换后的
    /// 当前会话，run_id 每次出站新生成，不复用旧值。
    pub fn take_prompt(&mut self) -> Option<PromptDelivery> {
        if self.running {
            return None;
        }
        let text = self.outgoing.pop_front()?;
        // T10：出队即释放字节账（与入队 [`Self::enqueue`]、召回
        // [`Self::recall_latest`] 的加减对称——总量上限才不会越记越紧）。
        self.queue_bytes = self.queue_bytes.saturating_sub(text.len());
        self.running = true;
        self.status.clear();
        // T13：出站 = 转为运行中，确认态自动取消（route §3 触发裁决：
        // 转为运行中时自动取消确认）。置位在 `status.clear()` 之后——取消
        // 回执要留下可读；结构上不可达（模态盖键 + [`Self::start_rewind`]
        // 已拒空队列），防守性落点：确认态绝不许跟到运行中的台上去。
        if self.rewind_confirm.take().is_some() {
            self.status = "rewind cancelled — turn running".to_string();
        }
        self.run_started = Some(Instant::now());
        self.ui.push_message(user_message(&text));
        let session_id = self.session_id.clone();
        let run_id = self.fresh_run_id();
        self.current_run = Some(run_id.clone());
        Some(PromptDelivery {
            session_id,
            run_id,
            text,
        })
    }

    /// 当前会话 id（prompt 路由的 session 字段）。
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// 当前在飞 run（`None` = 无）。
    pub fn current_run(&self) -> Option<&str> {
        self.current_run.as_deref()
    }

    /// 事件准入是否已切到 run 作用域（切过会话即为 `true`）。
    pub fn run_scoped(&self) -> bool {
        self.run_scoped
    }

    /// 起始会话（事件循环入口）：灌入 id 与历史，不切 run 作用域——
    /// 首个会话仍走 T2 的会话级事件流语义。
    pub fn start_session(&mut self, sid: &str, history: Vec<ChatMessage>) {
        self.adopt_session(sid, history);
    }

    /// 切会话（route §3 T4 红线）：新 session_id + `load_messages` 历史
    /// 回填 + 视图/出站队列清空 + 事件准入切 run 作用域——旧 run 的帧
    /// 从此进不了新会话视图，排队中的 prompt 也不许跟着搬进新会话。
    pub fn switch_session(&mut self, sid: &str, history: Vec<ChatMessage>) {
        self.adopt_session(sid, history);
        self.run_scoped = true;
    }

    /// run 作用域事件准入（T4 红线）：`run_id` 与当前 run 不符 → 整帧
    /// 丢弃。事件循环把 run 流订阅时用的 run_id 原样传入，旧 run（含切
    /// 会话前那条流）的事件因此到不了新视图。
    pub fn apply_run_event(&mut self, run_id: &str, ev: &AgentEvent) {
        if self.current_run.as_deref() != Some(run_id) {
            return;
        }
        self.apply_event(ev);
    }

    /// 历史滚动位置（0 = 钉底，最新在视）。
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// 历史滚动（正 = 向上翻更旧）。边界 = 回填后的历史条数
    /// `[0, len-1]`：越界夹紧、空历史恒 0，不许 usize 下溢 panic
    /// （route §4 `history_scroll_bounds_clamp`）。
    pub fn scroll_by(&mut self, delta: i32) {
        let max = self.ui.messages.len().saturating_sub(1);
        let next = self.scroll as i64 + i64::from(delta);
        self.scroll = next.clamp(0, max as i64) as usize;
    }

    /// transcript 视口滚动模型（T6 只读观察：翻页位 / 跟尾三态 / `↑N 行`）。
    pub fn viewport(&self) -> &ScrollModel {
        &self.viewport
    }

    /// 视口模型的每帧几何喂入缝（`ui::sync_viewport` 在 draw 前调用；
    /// 测试走同一入口，不绕开模型自己算边界）。
    pub(crate) fn viewport_mut(&mut self) -> &mut ScrollModel {
        &mut self.viewport
    }

    /// T7：滚轮入队（事件循环把 `Event::Mouse` 的 ScrollUp/ScrollDown 折成
    /// `dir = -1 / +1` 传进来）。每 notch 入队 [`WHEEL_LINES_PER_TICK`] 行，
    /// 总量 clamp [`WHEEL_QUEUE_MAX`] 防雪崩。
    ///
    /// 判窗与流（grok `on_scroll_event_at` 的归一化口径，refs `mouse.rs:666-677`）：
    /// - 间隔 ≤ [`WHEEL_TICK_DETECT_MAX_MS`]：同一物理 tick 批次（终端一批
    ///   多报 / 手势抖动）→ 归并进同一队列——同向累加，反向就地对消；
    /// - 出判窗的反向：换向即新流，**先清掉残留反向队列**（grok
    ///   `cancel_backlog`——反转必须即刻生效，不许先播一段反向余量），
    ///   再入队；
    /// - 同向跨 [`STREAM_GAP_MS`]（grok `gap > STREAM_GAP` 的新流）：本版
    ///   单队列折算 = 余量不清、继续按缓出冲刷消化（不整段跳、也不丢用户
    ///   行数）；流边界的独立分支留给 trackpad 档（见常数注释）。
    ///
    /// 本方法只入队不推进视口——推进全在 [`Self::drain_wheel`]，由此保证
    /// 「一次事件不整段跳、分 tick 前进」。
    pub fn wheel_tick(&mut self, dir: i32, now: Instant) {
        if dir == 0 {
            return;
        }
        if let Some((last_at, last_dir)) = self.wheel_last {
            let gap = now.saturating_duration_since(last_at);
            // 出判窗的换向：清残留反向队列（同流换向与跨流残留同口径，
            // grok `cancel_backlog`；同向不在此列，见 doc 上第 3 条）。
            if dir != last_dir && gap > Duration::from_millis(WHEEL_TICK_DETECT_MAX_MS) {
                self.wheel_queue = 0;
            }
        }
        self.wheel_last = Some((now, dir));
        self.wheel_queue = (self.wheel_queue + dir * WHEEL_LINES_PER_TICK)
            .clamp(-WHEEL_QUEUE_MAX, WHEEL_QUEUE_MAX);
    }

    /// T7：缓出冲刷（jcode `mouse_scroll_drain_amount` 的 3/2/1，refs
    /// `navigation.rs:795-808`）：距上次冲刷 ≥ [`REDRAW_CADENCE_MS`] 才推进
    /// 一次，按队列余量取步长——余量 ≥6 → 3 行、≥3 → 2 行、否则 1 行；
    /// 返回本次实际滚动的**带符号行数**（0 = 节流未到点 / 队列已空）。
    ///
    /// 落点是 [`ScrollModel::scroll_lines`]：上滚即脱钩、下滚落 max 重挂，
    /// `↑N 行` 指示随 T6 既有逻辑自动出现/消失。事件循环每 50ms 醒来调它
    /// 一次作 drain tick（idle 时队列也能消化）；flush 下限用 `Instant`
    /// 记账，不动 `POLL`。
    pub fn drain_wheel(&mut self, now: Instant) -> i32 {
        if self.wheel_queue == 0 {
            return 0;
        }
        if let Some(at) = self.wheel_flushed
            && now.saturating_duration_since(at) < Duration::from_millis(REDRAW_CADENCE_MS)
        {
            return 0;
        }
        let queued = self.wheel_queue.unsigned_abs();
        let want = match queued {
            n if n >= 6 => 3,
            n if n >= 3 => 2,
            _ => 1,
        };
        let step = want.min(queued) as i32;
        let dir = self.wheel_queue.signum();
        self.viewport.scroll_lines(dir * step);
        self.wheel_queue -= dir * step;
        self.wheel_flushed = Some(now);
        dir * step
    }

    /// T7：滚轮待出行数（测试观察缝：入队 clamp / 缓出消化的账都在这）。
    pub fn wheel_queue(&self) -> i32 {
        self.wheel_queue
    }

    /// 落座一个会话（起始与切换共用）：视图替换为回填历史、队列/状态
    /// 归零、滚动回到底。
    fn adopt_session(&mut self, sid: &str, history: Vec<ChatMessage>) {
        self.session_id = sid.to_string();
        self.ui = UiState::default();
        for msg in history {
            self.ui.push_message(msg);
        }
        self.outgoing.clear();
        // T10：排队 prompt 不许跟着搬进新会话（连同字节账一起归零）。
        self.queue_bytes = 0;
        self.running = false;
        self.current_run = None;
        self.status.clear();
        self.confirm_quit = false;
        self.scroll = 0;
        // T12：焦点/改写挂起都是旧台的坐标，随视图与出站队列（上面的
        // `outgoing.clear()` 连同 retry 的重发项一起丢弃）同批作废——留下
        // 截断点会在新台出站时按旧会话的 seq 截错会话。
        self.focused = None;
        self.edit_pending = None;
        self.truncate_pending = None;
        // T13：确认态/待执行回退同样是旧台的坐标（回退点 from_seq 属于
        // 旧会话），随视图同批作废——留着会在新台按旧会话的 seq 回退错台
        //（与 truncate_pending 同批口径）。
        self.rewind_confirm = None;
        self.rewind_pending = None;
        // T11：切台 = 视图整体更换——搜索命中/视口快照随旧台作废，overlay
        // 一并收掉（不走 [`Self::close_search`] 的还原语义：旧台的滚动
        // 位置没有意义，下面的 reset 才是新台基准）。
        self.forget_search();
        self.viewport.reset();
        // T7：上一台的滚轮待出行数不许滚进新会话视图（否则新会话钉底刚
        // 回填就被旧队列拽着脱钩）；冲刷节流记账保留（pacing 是全局的）。
        self.wheel_queue = 0;
        self.wheel_last = None;
    }

    /// 新一版 run_id：`r-<epoch_ms>`（与 T1/CLI 同格式），并按进程内时钟
    /// 单调递增——同毫秒连发也保证「切会话后 = 新 run_id」。
    fn fresh_run_id(&mut self) -> String {
        self.run_clock = now_epoch_ms().max(self.run_clock + 1);
        format!("r-{}", self.run_clock)
    }

    /// 本轮投影（`persist_assistant` 落库读它）。
    pub(crate) fn ui_state(&self) -> &UiState {
        &self.ui
    }

    /// 一次按键路由（route §3 键位契约；非 Press/Repeat 直接忽略）。
    pub fn handle_key(&mut self, key: KeyEvent) -> KeyAction {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return KeyAction::None;
        }
        // T13：rewind 确认态是**模态**（route §3 触发裁决）——先于一切路由
        // 收口：搜索 overlay、斜杠面板、退出确认、中断、编辑键全部让位，
        // 键只归 [`Self::rewind_confirm_key`]（y/Enter 确认、n/Esc/Ctrl+C
        // 取消、其余一律忽略）。确认态不可能与 copy 短反馈同框（进确认态
        // 时状态行已被 `discard ...` 覆写），故早于下面的短反馈清理。
        if self.rewind_confirm.is_some() {
            return self.rewind_confirm_key(key);
        }
        // T12：copy 短反馈只活到下一次按键（route §3 注记①「随下一次按键/
        // 状态事件恢复常态」）——先清再路由，本次按键该置的照置。
        if self.status == COPIED_STATUS {
            self.status.clear();
        }
        // T11：搜索 overlay 打开 = 整段短路（最前分派，早于斜杠面板与一切
        // composer/退出/滚动路由——overlay 期间没有 composer 语义，按键
        // 全部在 [`Self::search_key`] 收口，不产生退出/中断/入队副作用）。
        if self.search_open() {
            return self.search_key(key);
        }
        // T9：面板可见时的短路路由（dh-rs 分流结构，任务书 §2 实录）：导航 /
        // 第一段补全 / Esc 关面板在这里**早 return**——补全与执行被
        // [`Self::slash_complete`] 的单臂出口隔开，fencing 结构上不可能双触发。
        // 编辑键、ctrl 键、翻页键不拦截，落回下面的既有路由。
        if self.slash_visible()
            && let Some(action) = self.slash_key(key)
        {
            return action;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            // 退出/中断双键与确认态：先于一切编辑路由。
            KeyCode::Esc => return self.on_escape(),
            KeyCode::Char('c') if ctrl => return self.on_escape(),
            // Ctrl+D 只在 composer 为空时是退出（非空 = 忽略，不吞输入）。
            KeyCode::Char('d') if ctrl && self.input.is_empty() => return KeyAction::Quit,
            KeyCode::Char('d') if ctrl => return KeyAction::None,
            // T11：Ctrl+R 开搜索 overlay（route §3 T11 双入口之一，与
            // `/search` 同落点 [`Self::open_search`]；总闸关 = no-op，
            // 键位与合入前一致）。落回普通字符前拦，别让它进 composer。
            KeyCode::Char('r' | 'R') if ctrl => {
                self.open_search();
                return KeyAction::None;
            }
            _ => {}
        }
        // 其余任何键先撤掉未决退出确认（提示行承诺 other key cancels）。
        self.confirm_quit = false;
        // T3 问题面板可见：数字键优先答题（route §3——按数字键发送
        // {id, choice}；越界 = 忽略，吞掉该键不落进 composer）。
        if !self.questions.is_empty()
            && !ctrl
            && let KeyCode::Char(c @ '1'..='9') = key.code
        {
            if let Some(request) = self.questions.press_digit(c) {
                self.answers.push_back(request);
            }
            return KeyAction::None;
        }
        // T12：聚焦态的 r/e/c 在这里收口（route §3 注记④入口仲裁）——聚焦
        // 才拦、非聚焦原样落回 composer 路由（三键零打字干扰）；运行中/
        // 队列非空的互斥短路只在 [`Self::message_key`] 一处。
        if self.message_ops_enabled
            && self.focused.is_some()
            && !ctrl
            && let Some(action) = self.message_key(key)
        {
            return action;
        }
        match key.code {
            KeyCode::Enter => self.submit_line(),
            // T12：Shift+Tab 前向环绕聚焦下一条 user/assistant 消息（正常
            // 模式该键原本空闲；面板可见时已归 [`Self::slash_key`]）。Tab
            // 照旧翻工具卡（route §3 注记④：Tab 不动）。
            KeyCode::BackTab if self.message_ops_enabled => self.focus_next(),
            // T3：一个键全部展开/折叠工具卡（route §3；Tab 不进 composer）。
            KeyCode::Tab => self.tools_expanded = !self.tools_expanded,
            KeyCode::Backspace => {
                self.input.pop();
                self.slash_edited();
                self.history_pos = None;
            }
            // T6：transcript 视口键（route §3 T6）。PgUp/PgDn/End 与 composer
            // 无关——单行输入没有翻页/到头语义（光标恒在行尾），不吞编辑；
            // Ctrl+U 半页只在 composer 为空时归滚动，有文本落回编辑语义
            // （现有编辑不认 Ctrl+U = 忽略不吞字，jcode 仲裁口径）。↑↓ 历史
            // 行为照旧（contract §3 键位仲裁第一条），仅 T10 加一条前置分叉：
            // 队列非空且 composer 为空时 ↑ 先召回排队最新项（[`Self::recall_latest`]）。
            KeyCode::PageUp => self.viewport.page_up(),
            KeyCode::PageDown => self.viewport.page_down(),
            KeyCode::End => self.viewport.to_end(),
            KeyCode::Char('u') if ctrl && self.input.is_empty() => self.viewport.half_up(),
            // T10：首行 ↑ 召回（route §3）——队列非空且 composer 为空才召回，
            // 否则原样落回历史导航（composer 非空 = 让位编辑/历史，不吞键）。
            KeyCode::Up => {
                if !self.recall_latest() {
                    self.history_back();
                }
            }
            KeyCode::Down => self.history_forward(),
            KeyCode::Char(c) if !ctrl => {
                self.input.push(c);
                self.slash_edited();
                self.history_pos = None;
            }
            _ => {}
        }
        // T12：composer 被清空 = 放弃改写挂起——edit 的截断只随提交消费，
        // 半路清空不许在之后一次无关发送里把历史尾段截掉（route §3 注记③
        // 只定了「提交时消费」，放弃这一半由这里补上判据）。
        self.retire_stale_edit();
        KeyAction::None
    }

    /// Esc / Ctrl+C：运行中 = Abort；焦点存在 = 清焦；空闲 = 第一下要确认、
    /// 第二下才 Quit。
    ///
    /// T12 优先级（route §3 注记④）：搜索 overlay 与斜杠面板在
    /// [`Self::handle_key`] 更早的分派里各自收口 Esc，落到这里的顺序是
    /// **清焦 → 退出确认**——焦点在，第一下 Esc 只清焦、不进退出确认。
    /// 运行中照旧 Abort（中断契约优先级高于清焦，焦点不改中断语义）。
    fn on_escape(&mut self) -> KeyAction {
        if self.running {
            return KeyAction::Abort;
        }
        if self.focused.take().is_some() {
            return KeyAction::None;
        }
        if self.confirm_quit {
            return KeyAction::Quit;
        }
        self.confirm_quit = true;
        KeyAction::None
    }

    /// T12：聚焦态 `r`/`e`/`c` 的收口——返回 `Some` = 已拦截（聚焦时这三键
    /// **永远不该落进 composer**，被互斥挡下也是吞键 no-op），`None` = 不归
    /// 聚焦路由管（非 r/e/c、带修饰键），落回既有编辑键路由。
    ///
    /// 运行中/队列非空的互斥**只在这一处**（route §3 注记⑤）：重跑会打乱
    /// T10 队列的逐条消费次序；copy 虽无状态变更也同禁（运行中活动行被
    /// running 占用，`copied` 反馈无处安放）。单点 fencing 消除三键各查
    /// 各的歧义，`tests/message_actions.rs` 钉死。
    fn message_key(&mut self, key: KeyEvent) -> Option<KeyAction> {
        let KeyCode::Char(c) = key.code else {
            return None;
        };
        if !matches!(c, 'r' | 'e' | 'c')
            || key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return None;
        }
        if self.running || !self.outgoing.is_empty() {
            return Some(KeyAction::None);
        }
        // 一次只许一笔改写挂起：edit 的截断点到提交才消费，半路再挂一笔会让
        // 两个截断点互相覆盖。copy 不动历史，不受此限。
        if self.edit_pending.is_some() && c != 'c' {
            self.set_status("pending edit — enter submits, clear the composer to discard");
            return Some(KeyAction::None);
        }
        match c {
            'r' => self.retry_focused(),
            'e' => self.edit_focused(),
            _ => self.copy_focused(),
        }
        Some(KeyAction::None)
    }

    /// T12：BackTab 前向环绕聚焦（route §3 注记④）——只在 user/assistant
    /// 消息间移动（本地 `system` 输出不是消息，不可操作）；无焦点时先落
    /// **最新一条**（聚焦是为了操作刚读到的内容），到尾回绕到头。
    fn focus_next(&mut self) {
        let msgs = &self.ui.messages;
        let n = msgs.len();
        let eligible = |i: usize| matches!(msgs[i].role.as_str(), "user" | "assistant");
        let found = match self.focused {
            None => (0..n).rev().find(|&i| eligible(i)),
            Some(i) => (i + 1..i + 1 + n).map(|k| k % n).find(|&i| eligible(i)),
        };
        let Some(next) = found else {
            return;
        };
        self.focused = Some(next);
        self.set_status("focused — r retry · e edit · c copy · esc clears");
    }

    /// T12/T13：消息 id → ledger seq，口径 = `{session_id}-{seq}`
    /// （[`Self::note_appended_seq`] 与 `message_to_chat` 同一套）；解析不出
    /// 或非正数 = 无从定位序号，返回 `None`（不猜序号，宁可不动作也不截
    /// 错段）。retry/edit 与 rewind 的回退点共用这一个解析。
    fn message_seq(&self, msg: &ChatMessage) -> Option<i64> {
        let seq: i64 = msg
            .id
            .strip_prefix(&self.session_id)?
            .strip_prefix('-')?
            .parse()
            .ok()?;
        if seq <= 0 {
            return None;
        }
        Some(seq)
    }

    /// T12：retry/edit 的改写目标 =（消息下标, ledger seq, 可见文本）。只认
    /// 聚焦的 **user** 消息，且 id 能解析出本会话序号（[`Self::message_seq`]）
    /// ——解析不出即 `None`，理由见该方法。
    fn rewrite_target(&self) -> Option<(usize, i64, String)> {
        let i = self.focused?;
        let msg = self.ui.messages.get(i)?;
        if msg.role != "user" {
            return None;
        }
        let seq = self.message_seq(msg)?;
        Some((i, seq, visible_text(msg)))
    }

    /// T12：`r` 重发聚焦 user 消息的原文（route §3 注记②）——原文**入既有
    /// 出站队列**（复用 [`Self::take_prompt`] 起的 `PromptDelivery` 三元组，
    /// 不新开发送路径），截断点同步挂给事件循环：出站时先
    /// `session.truncate(from_seq)` 再落库，重发的那条才不会把自己删掉。
    /// 视图尾段此刻裁掉（与 ledger 截断同界，见 [`Self::drop_tail_from`]）。
    fn retry_focused(&mut self) {
        let Some((keep, seq, text)) = self.rewrite_target() else {
            self.set_status("retry: focus a user message of this session");
            return;
        };
        if text.trim().is_empty() {
            return;
        }
        if let Err(reason) = self.enqueue(&text) {
            // 队列上限拒绝照 T10 口径可见（64KiB 超长原文重发会被拒）——
            // 拒收就不挂截断点，历史原样保留。
            self.set_status(reason);
            return;
        }
        self.truncate_pending = Some(seq);
        self.drop_tail_from(keep);
    }

    /// T12：`e` 改写聚焦 user 消息（route §3 注记③）——该条文本回填
    /// composer（复用 composer 全部编辑能力，零新编辑 UI）。截断**不在**这
    /// 一刻：`edit_pending` 挂着 `(下标, seq)`，提交才消费（见
    /// [`Self::submit_line`]）；半路清空 composer 即放弃（见
    /// [`Self::retire_stale_edit`]），历史不动。视图里那条留着（尚未截断），
    /// 焦点让位给 composer 编辑。
    fn edit_focused(&mut self) {
        let Some((keep, seq, text)) = self.rewrite_target() else {
            self.set_status("edit: focus a user message of this session");
            return;
        };
        if text.trim().is_empty() {
            return;
        }
        self.input = text;
        self.history_pos = None;
        self.edit_pending = Some((keep, seq));
        self.focused = None;
        self.set_status("edit — enter submits the revision");
    }

    /// T12：`c` 复制聚焦消息的可见文本（route §3 注记①：范围 = 单条
    /// **assistant** 文本，选择模式已 Out of scope）——正文进 `copies` 队列，
    /// 事件循环取出写 OSC52；反馈置 `copied`（只陈述「本方已写 stdout」这一
    /// 已证明事实，终端是否采纳不可证）。
    fn copy_focused(&mut self) {
        let Some(msg) = self
            .focused
            .and_then(|i| self.ui.messages.get(i))
            .filter(|m| m.role == "assistant")
        else {
            self.set_status("copy: focus an assistant message");
            return;
        };
        let text = visible_text(msg);
        if text.is_empty() {
            self.set_status("nothing to copy");
            return;
        }
        self.copies.push_back(text);
        self.set_status(COPIED_STATUS);
    }

    /// T12：把本地投影裁到 `keep` 条——retry/edit 的视图侧与 ledger 截断
    /// 同界（视图在按键/提交时裁，RPC 在出站时截，两步之间只隔一次事件
    /// 循环迭代）。焦点随被裁消息作废，搜索命中/视口快照同步失效：与
    /// [`Self::clear_view`] 共用同一个失效源 [`Self::forget_search`]，
    /// 不另写一份清理。
    fn drop_tail_from(&mut self, keep: usize) {
        if self.ui.messages.len() > keep {
            self.ui.messages.truncate(keep);
            self.focused = None;
            self.forget_search();
        }
    }

    /// T12：composer 被清空 = 放弃改写挂起（[`Self::handle_key`] 每次按键
    /// 末尾调）——edit 的截断只随提交消费，半路清空必须撤下挂起，否则之后
    /// 一次无关发送会把历史尾段截掉（route §3 注记③只定了「提交时消费」，
    /// 放弃这一半由这里补上判据）。retry 不受影响：它没挂 `edit_pending`
    /// （截断点与已入队的重发绑定），本函数不碰。
    fn retire_stale_edit(&mut self) {
        if self.edit_pending.is_some() && self.input.is_empty() {
            self.edit_pending = None;
            self.set_status("edit discarded");
        }
    }

    // --- T13 逐轮回退（route §3 T13 设计注记②③④ + 触发裁决）：`/rewind [n]`
    // 执行只进确认态，y/Enter 才产出动作；回退点复用 T12 的 id→seq 口径 ---

    /// T13：`/rewind [n]` 执行入口（两段 Enter 的第二段走到这里）——互斥
    /// 拒绝、解析 n、算回退点，全过了才进确认态；任何拒绝都显式报状态行、
    /// **零动作**（不进确认态、不动视图、不挂 pending）。
    fn start_rewind(&mut self, arg: &str) {
        // 互斥（route §3 注记：T10 队列逐放期间 ledger 在变、回退点不可信）：
        // running 或出站非空时执行即拒——fencing 思路同 T12 [`Self::message_key`]
        // 的单点短路，但入口在斜杠执行路径，不是焦点键。
        if self.running || !self.outgoing.is_empty() {
            self.set_status("rewind unavailable while running or queued");
            return;
        }
        // n 缺省 1；非整数 → 显式报错零动作。
        let n: i64 = if arg.is_empty() {
            1
        } else {
            match arg.parse() {
                Ok(n) => n,
                Err(_) => {
                    self.set_status(format!(
                        "rewind: expected an integer turn count, got {arg:?}"
                    ));
                    return;
                }
            }
        };
        let targets = self.user_message_seqs();
        let count = targets.len() as i64;
        if count == 0 {
            self.set_status("rewind: no user turns in this session");
            return;
        }
        // 校验 n ∈ [1, count]（route §3 注记②：越界/零 → 显式报错、零动作）。
        if n < 1 || n > count {
            self.set_status(format!("rewind: n must be between 1 and {count}"));
            return;
        }
        // 回退点 = 倒数第 n 条 user 消息的 seq；确认态显示的轮数 = 已校验的 n。
        let (_, from_seq) = targets[(count - n) as usize];
        self.rewind_confirm = Some((from_seq, n));
        self.set_status(format!("discard {n} turn(s)? [y/n]"));
    }

    /// T13：本会话可定位的 user 消息（视图顺序，`(下标, seq)`）——n 校验与
    /// 回退点计算同源；`system` 本地输出与 id 解析不出 seq 的消息不计（同
    /// T12 [`Self::rewrite_target`] 的宁缺勿错口径）。
    fn user_message_seqs(&self) -> Vec<(usize, i64)> {
        self.ui
            .messages
            .iter()
            .enumerate()
            .filter(|(_, msg)| msg.role == "user")
            .filter_map(|(i, msg)| self.message_seq(msg).map(|seq| (i, seq)))
            .collect()
    }

    /// T13：确认态 y/Enter——把**已校验**的 `(from_seq, n)` 转成动作（RPC
    /// 归事件循环，App 无 IO），提示行让位给结果行（`rewound ...` /
    /// `rewind failed: ...` 由回填置）。
    fn confirm_rewind(&mut self) {
        let Some(pending) = self.rewind_confirm.take() else {
            return;
        };
        self.rewind_pending = Some(pending);
        self.status.clear();
    }

    /// T13：确认态 n/Esc/Ctrl+C——取消**零动作**（回退点、视图、ledger 全都
    /// 不动，只留一行可见的取消回执）。
    fn cancel_rewind(&mut self) {
        self.rewind_confirm = None;
        self.set_status("rewind cancelled");
    }

    /// T13：确认态按键收口（[`Self::handle_key`] 最先分派，模态盖过搜索
    /// overlay、斜杠面板与退出确认——route §3 触发裁决「其余键一律忽略」）：
    /// y/Enter 确认、n/Esc/Ctrl+C 取消（Ctrl+C 的中断/退出语义在确认态让位
    /// 给取消），其余键一律忽略（Ctrl+R 不开搜索、Ctrl+D 不退出、编辑键
    /// 不落 composer）。
    fn rewind_confirm_key(&mut self, key: KeyEvent) -> KeyAction {
        let plain = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        match key.code {
            KeyCode::Char('y' | 'Y') if plain => self.confirm_rewind(),
            KeyCode::Char('n' | 'N') if plain => self.cancel_rewind(),
            KeyCode::Enter => self.confirm_rewind(),
            KeyCode::Esc => self.cancel_rewind(),
            KeyCode::Char('c') if !plain => self.cancel_rewind(),
            _ => {}
        }
        KeyAction::None
    }

    /// T13：`session.rewind` 成功回填——transcript 强制对齐回退后的 ledger：
    /// 本地投影整体换成 `history`（走切会话同一条 [`Self::switch_session`]
    /// 重载路径——视图/滚动/焦点/搜索快照同批重置），状态行
    /// `rewound n turn(s)`。
    pub fn note_rewind_ok(&mut self, n: i64, history: Vec<ChatMessage>) {
        let sid = self.session_id.clone();
        self.switch_session(&sid, history);
        self.set_status(format!("rewound {n} turn(s)"));
    }

    /// T13：`session.rewind` 失败回填（真值安全）——状态行
    /// `rewind failed: <reason>`，本地投影一个字节都不动：宁可见不一致，
    /// 也不假装回退成功（route §3 注记④）。
    pub fn note_rewind_failed(&mut self, reason: impl Into<String>) {
        self.set_status(format!("rewind failed: {}", reason.into()));
    }

    /// T9：面板可见时的短路键——返回 `Some` = 已拦截（[`Self::handle_key`]
    /// 早 return），`None` = 不归面板管，落回既有路由（编辑键、ctrl 键、
    /// 翻页键都在那一侧）。
    fn slash_key(&mut self, key: KeyEvent) -> Option<KeyAction> {
        match key.code {
            // Esc：只关面板（composer 原文保留）；不进退出确认、不碰中断
            // 路由——想 abort 再按一次（面板已关）就回到既有语义。
            KeyCode::Esc => {
                self.slash_suppressed = true;
                Some(KeyAction::None)
            }
            // 第一段 Enter：选中项补全进 composer（执行是第二段的事）。
            KeyCode::Enter => self.slash_complete(),
            KeyCode::Up | KeyCode::BackTab => {
                self.slash_move(-1);
                Some(KeyAction::None)
            }
            // Tab 归面板导航（面板可见时不翻工具卡，关面板后恢复原路由）。
            KeyCode::Down | KeyCode::Tab => {
                self.slash_move(1);
                Some(KeyAction::None)
            }
            _ => None,
        }
    }

    /// T9：第一段 Enter——补全高亮项并抑制面板，**只补全不执行**：本臂的
    /// 早 return 就是 fencing（第二段 Enter 面板已关，才走 [`Self::submit_line`]
    /// 执行）。无命中（如 `/zz`）不拦截：落回提交路由报「未知命令」，不发消息。
    fn slash_complete(&mut self) -> Option<KeyAction> {
        let matches = self.slash_matches();
        let selected = matches.get(self.slash_cursor(matches.len()))?;
        // T13：补全保留参数尾巴（`/rewind 3` 第一段 Enter 后仍是
        // `/rewind 3`，不把已敲的 n 抹掉）——无尾巴时补全到命令名本身，
        // 行为与合入前逐字一致。
        let tail = self
            .input
            .split_once(char::is_whitespace)
            .map(|(_, rest)| rest.trim())
            .unwrap_or("");
        self.input = if tail.is_empty() {
            selected.name.to_string()
        } else {
            format!("{} {tail}", selected.name)
        };
        self.slash_suppressed = true;
        self.slash_selected = 0;
        Some(KeyAction::None)
    }

    /// T9：面板游标移动（候选空 = no-op；到顶/到底夹紧，不环绕）。
    fn slash_move(&mut self, delta: i32) {
        let len = self.slash_matches().len();
        if len == 0 {
            return;
        }
        let cursor = self.slash_cursor(len) as i32;
        self.slash_selected = cursor.saturating_add(delta).clamp(0, len as i32 - 1) as usize;
    }

    /// T9：composer 编辑 → 面板复位（解除抑制重新武装 + 游标回顶）。
    fn slash_edited(&mut self) {
        self.slash_suppressed = false;
        self.slash_selected = 0;
    }

    /// Enter：面板可见时第一段只补全（[`Self::slash_complete`]）；否则——
    /// 整行 trim 后命中注册表 → **本地执行**（不进 prompt 提交路径、不产生
    /// 模型回合，route §3 T9）；行首未知 `/` → 状态行报未知命令、原文留在
    /// composer（同样不发消息）；其余照旧：非空进历史 + 出站队列，空行只清行
    /// （route §3：Enter 空串不提交——空行误发由这里挡）。T10：入队先过
    /// 双上限（[`Self::enqueue`]），超限原文退回 composer + 状态行明说。
    fn submit_line(&mut self) {
        let text = std::mem::take(&mut self.input);
        self.history_pos = None;
        self.slash_suppressed = false;
        self.slash_selected = 0;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        if self.slash_enabled {
            // T13：整行解析改走 [`slash::find_line`]——精确命中照旧（无参
            // 命令语义与 T9 逐字一致），带参行（`/rewind 3`）拆出首 token
            // 与参数尾巴一并交给执行。
            if let Some((cmd, arg)) = slash::find_line(trimmed) {
                self.run_command(cmd, arg);
                return;
            }
            if trimmed.starts_with('/') {
                // 未知斜杠行：不发消息，原文留给用户改，活动行可见。
                let status = format!("unknown command: {trimmed}");
                self.input = text;
                self.set_status(status);
                return;
            }
        }
        if let Err(reason) = self.enqueue(&text) {
            // T10：拒收不静默丢——原文退回 composer（dh-rs 超限保草稿同款）、
            // 状态行明说原因；不进历史（没提交成功的东西不算已发）。
            self.input = text;
            self.set_status(reason);
            return;
        }
        // T12：edit 的改写在提交这一刻消费（route §3 注记③）——截断点挂给
        // 事件循环（出站时先 truncate 再落库改文），视图同步裁到改写点。
        // 只有真正入队成功才动历史：拒收时挂起保留（原文已退回 composer，
        // 用户改完还能提交）。
        if let Some((keep, seq)) = self.edit_pending.take() {
            self.truncate_pending = Some(seq);
            self.drop_tail_from(keep);
        }
        self.history.push(text);
    }

    /// T10：文本入出站队列（FIFO；dh-rs `PromptQueue::enqueue_from` 同构的
    /// 双上限判定——条数 [`MAX_QUEUE_ITEMS`] 满员拒收、总量 [`MAX_QUEUE_BYTES`]
    /// 超额拒收，先算后入不等溢出）。超限返回状态行文案，由调用方
    /// [`Self::submit_line`] 把原文退回 composer 并落状态（route §3：
    /// 上限拒绝要看得见，不许静默丢）。
    fn enqueue(&mut self, text: &str) -> Result<(), String> {
        if self.outgoing.len() >= MAX_QUEUE_ITEMS {
            return Err(format!("queue full: {MAX_QUEUE_ITEMS} prompts max"));
        }
        if self.queue_bytes.saturating_add(text.len()) > MAX_QUEUE_BYTES {
            return Err(format!(
                "queue full: {}KiB total max",
                MAX_QUEUE_BYTES / 1024
            ));
        }
        self.outgoing.push_back(text.to_string());
        self.queue_bytes += text.len();
        Ok(())
    }

    /// T9：本地执行一条注册命令（首批全是本地/面板语义）：不入出站队列、
    /// 不碰 prompt 提交路径——命令执行不产生模型回合。
    ///
    /// T13：`arg` 是 [`slash::find_line`] 拆出的参数尾巴——无参命令恒为空串
    /// （`/help extra` 这类带尾巴的无参行在 `find_line` 就落空，到不了这），
    /// 只有 `/rewind [n]` 用它。
    fn run_command(&mut self, cmd: &slash::Command, arg: &str) {
        match cmd.action {
            // 输出遍历注册表生成（面板与 /help 同源，见 `slash::help_text`）。
            Action::Help => self.push_local(&slash::help_text()),
            // 回显 footer 同源的运行时配置（`footer::configured_model`）：
            // TUI 没有模型选择能力，只报真值不编造。
            Action::Model => {
                let model = if self.model.is_empty() {
                    "unset".to_string()
                } else {
                    self.model.clone()
                };
                self.push_local(&format!("model: {model}"));
            }
            // 主题能力现状回显：TUI 只有一套内置命名色（D11），切换是 T17
            // 的活——编不到就明说没有切换能力。
            Action::Theme => self.push_local(THEME_LINE),
            // 视图清屏（只清本地投影；运行中禁——落库源就在这张表里）。
            Action::Clear => {
                if self.running {
                    self.set_status("clear unavailable while running");
                } else {
                    self.clear_view();
                }
            }
            // 会话导航：入意图队列，picker 由事件循环开（App 无 IO）。
            Action::Sessions => {
                if self.running {
                    self.set_status("sessions unavailable while running");
                } else {
                    self.intents.push_back(Intent::OpenSessions);
                }
            }
            // T11：转录搜索 overlay（与 Ctrl+R 同一落点——双入口等效由
            // `tests/search_overlay.rs` 钉；总闸关 = no-op）。
            Action::Search => self.open_search(),
            // T13：执行只进确认态（互斥拒绝/解析 n/算回退点都在
            // [`Self::start_rewind`]，任何拒绝零动作），y/Enter 才产出动作。
            Action::Rewind => self.start_rewind(arg),
        }
    }

    /// T9：`/clear` 清本地视图（route §3「视图清屏」）：只清 transcript 投影
    /// 与滚动，daemon 侧会话数据不动（回填走切会话的 `load_history`）。
    fn clear_view(&mut self) {
        self.ui = UiState::default();
        self.scroll = 0;
        // T11：命中行号与视口快照全部作废，overlay 一并收掉（视图清空后
        // 旧命中/旧快照都没有意义）。
        self.forget_search();
        // T12：视图清空后焦点下标与改写挂起的坐标同样失效（截断点到下一次
        // 提交才消费，留着会按已删掉的下标/seq 截没影的历史）。
        self.focused = None;
        self.edit_pending = None;
        self.truncate_pending = None;
        self.viewport.reset();
    }

    /// T9：一条本地输出进 transcript（`system` 角色：与 user/assistant 区分，
    /// 渲染走无前缀正文；`persist_assistant` 只挑 `assistant`，它永不落库）。
    ///
    /// 流式进行中 `messages.last()` 必须保持 `assistant`——`UiState::apply`
    /// 的 `last_assistant_or_placeholder` 按最后一条的 role 决定增量落在哪，
    /// 追加到它后面会把本轮回复劈成两条（后半截丢落库）。故本轮在跑且最后
    /// 一条是 assistant 时插到它前面；其余情形直接追加。
    fn push_local(&mut self, text: &str) {
        let msg = local_message(text);
        let streaming = self.running
            && self
                .ui
                .messages
                .last()
                .is_some_and(|m| m.role == "assistant");
        if streaming {
            let at = self.ui.messages.len() - 1;
            self.ui.messages.insert(at, msg);
        } else {
            self.ui.push_message(msg);
        }
    }

    /// T10：首行 ↑ 召回——取队列**最新项**（队尾，dh-rs `recall_latest`
    /// 口径）放进编辑区，其余队列项保持 FIFO 顺序不重排，字节账同步减。
    /// 条件 = 队列非空且 composer 为空（单行 composer 无多行，空即首行）；
    /// 不满足返回 `false`，↑ 由 [`Self::handle_key`] 落回既有历史导航。
    fn recall_latest(&mut self) -> bool {
        if self.outgoing.is_empty() || !self.input.is_empty() {
            return false;
        }
        let Some(text) = self.outgoing.pop_back() else {
            return false;
        };
        self.queue_bytes = self.queue_bytes.saturating_sub(text.len());
        self.input = text;
        true
    }

    /// ↑：往回翻历史（首触时暂存草稿）；到最老一条停住，不越界。
    fn history_back(&mut self) {
        if self.history.is_empty() {
            return;
        }
        if self.history_pos.is_none() {
            self.draft = self.input.clone();
        }
        let newest = self.history.len() - 1;
        let pos = self.history_pos.map_or(0, |p| p + 1).min(newest);
        self.history_pos = Some(pos);
        self.input.clone_from(&self.history[newest - pos]);
    }

    /// ↓：往新翻；越过最新一条恢复草稿（丢编辑内容 / 越界 panic 由
    /// `tests/keys.rs::history_up_down_cycles_local_edits` 钉）。
    fn history_forward(&mut self) {
        match self.history_pos {
            None => {}
            Some(0) => {
                self.history_pos = None;
                self.input.clone_from(&self.draft);
            }
            Some(pos) => {
                let pos = pos - 1;
                self.history_pos = Some(pos);
                self.input
                    .clone_from(&self.history[self.history.len() - 1 - pos]);
            }
        }
    }
}

/// user 消息本地投影（内容与 role 同 `UiState` 的 web 形状）。
fn user_message(text: &str) -> ChatMessage {
    let now = now_epoch_ms();
    ChatMessage {
        id: format!("user-{now}"),
        role: "user".to_string(),
        content: text.to_string(),
        reasoning: String::new(),
        tool_calls: vec![],
        parts: vec![MessagePart::Text(text.to_string())],
        timestamp: String::new(),
        ts_epoch_ms: now,
        attachments: vec![],
    }
}

/// T9：本地命令输出的投影（`system` 角色——渲染走无前缀正文，
/// `persist_assistant` 的 assistant 过滤天然跳过它，永不落库）。
fn local_message(text: &str) -> ChatMessage {
    let now = now_epoch_ms();
    ChatMessage {
        id: format!("local-{now}"),
        role: "system".to_string(),
        content: text.to_string(),
        reasoning: String::new(),
        tool_calls: vec![],
        parts: vec![MessagePart::Text(text.to_string())],
        timestamp: String::new(),
        ts_epoch_ms: now,
        attachments: vec![],
    }
}

/// T12：消息的**可见文本**——copy 的正文与 retry 重发的原文同一口径，两者
/// 必须一字不差（复制出去的就该是重发出去的）。`parts` 非空 = 结构化投影，
/// 按 `MessagePart::Text` 顺序拼接（跳过 tool part：卡片是界面物，不是消息
/// 文本）；为空则回退 `content`（`message_to_chat` 回填的消息只有 content）。
/// 两者都拼会把同一篇正文复制两份（流式投影 content 与 parts 同源）。
fn visible_text(msg: &ChatMessage) -> String {
    if msg.parts.is_empty() {
        return msg.content.clone();
    }
    let texts: Vec<&str> = msg
        .parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Text(text) => Some(text.as_str()),
            MessagePart::Tool(_) => None,
        })
        .collect();
    if texts.is_empty() {
        msg.content.clone()
    } else {
        texts.join("\n")
    }
}

// --- T12 OSC52 剪贴板写回（route §3 注记①定案：载体 = OSC52，终端原生、
// 零 daemon 依赖、SSH 场景可用；系统剪贴板 Deferred） ---
//
// D11（`tests/theme_lint.rs` 扫全 `crates/tui/src`）：转义字面量与十六进制
// 数值字面量都在禁令里（裸字节与文本形态同罚）——ESC、BEL 一律用十进制
// const 拼装，base64 查表也只用十进制下标。

/// ESC（十进制 27）：OSC 转义序列的开场字节。
const OSC_ESC: u8 = 27;

/// BEL（十进制 7）：OSC52 序列的终止字节。
const OSC_BEL: u8 = 7;

/// T12：正文 → OSC52 序列字节（`ESC ] 52 ; c ; <base64> BEL`，`c` = 剪贴板
/// 选择，dsh/jcode 同款目标）。序列化为纯函数，测试直接钉字节形状。
pub fn osc52_bytes(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 16);
    out.push(OSC_ESC);
    out.extend_from_slice(b"]52;c;");
    out.extend_from_slice(base64_std(text.as_bytes()).as_bytes());
    out.push(OSC_BEL);
    out
}

/// T12：把一条 copy 写到 stdout（事件循环消费 [`App::take_copy`]）——写完
/// 刷缓冲，OSC52 才赶在下一帧重绘前进终端。
fn write_osc52(text: &str) -> Result<(), TuiError> {
    use std::io::Write as _;
    let mut out = std::io::stdout().lock();
    out.write_all(&osc52_bytes(text)).map_err(TuiError::Io)?;
    out.flush().map_err(TuiError::Io)
}

/// 标准 base64（RFC 4648 含 padding）——OSC52 的载荷格式。手写是为了不给
/// `Cargo.toml` 添依赖（route §1 白名单只放行改动文件，40 行的定长查表
/// 换一个新 crate 不划算）。
fn base64_std(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// enhanced 全屏外壳（route §3 签名，不许改）。
///
/// 行为契约：进 alternate screen + 启用鼠标捕获 + raw mode（[`TermGuard`]）；
/// 布局 = 上方
/// transcript + 底部 dock（composer/活动/排队/按键提示）；Enter 非空提交
/// （`worker_prompt_run`）、空串不提交；↑/↓ 本地历史；Esc/Ctrl+C 运行中
/// `worker.abort`、空闲确认退出；Ctrl+D 空 composer 退出；鼠标滚轮滚动
/// transcript（T7 [`App::wheel_tick`] / [`App::drain_wheel`]，仅本路径启
/// 捕获）；daemon 断线单行错误退出码 3；**所有退出路径**都经
/// [`TermGuard::leave`] 还原（含拆 mouse tracking），panic 走先装的 hook
/// （先还原再打印 + `$TMPDIR` 崩溃报告）。
///
/// `rx` = 调用方起好的会话级 worker 事件流；`opts` 解析会话（`--session`
/// 不存在 → 退出码 2，先于进屏，错误不落在 alternate screen 里）。
/// `--resume` 在进屏后打开会话 picker（[`crate::ui::session_picker`]，
/// route §3 T4）：选中即历史回填并切台，ESC 保持 resolve 出的原会话。
pub fn run_enhanced(
    opts: TuiOptions,
    client: WebDaemon,
    rx: Receiver<AgentEvent>,
) -> Result<(), TuiError> {
    let mut sid = resolve_session(&client, &opts)?;
    // panic hook 先于一切终端改动：随后任何 panic 都先还原再打印。
    crate::termguard::install_panic_hook();
    let mut guard = TermGuard::new(CrosstermOps);
    if let Err(e) = guard.enter() {
        // 进入可能只成功一半：留下的半截状态要收掉。
        let _ = guard.leave();
        return Err(TuiError::Io(e));
    }
    // T4：`--resume` → 会话 picker（进屏后画，ESC 返回 None = 保持
    // resolve_session 给的原会话；显式 `--session` 不进 picker）。
    let mut history = Vec::new();
    if opts.resume
        && opts.session.is_none()
        && let Some(picked) = crate::ui::session_picker::run_picker(&client, "")?
    {
        history = crate::ui::session_picker::load_history(&client, &picked)?;
        sid = picked;
    }
    let outcome = event_loop(&client, &sid, history, &rx);
    // 还原必执行（业务 Err 也走）；业务错误优先于还原错误。
    let leave = guard.leave();
    match (outcome, leave) {
        (Err(err), _) => Err(err),
        (Ok(()), Err(err)) => Err(TuiError::Io(err)),
        (Ok(()), Ok(())) => Ok(()),
    }
}

/// 全屏事件循环：出站 → draw → 按键 → 事件流 → T3 面板/footer 同步 →
/// prompt 结果，周而复始。
///
/// 事件准入两段式（route §3 T4 红线）：切会话前，会话级 `rx` 是视图事件
/// 源（T2 语义不变）；切会话后 `rx` 只做断线探测，视图事件改从当前 prompt
/// 的 run 作用域订阅进来——`subscribe_worker_run` 在源头按 run_id 丢掉旧
/// run / 其他会话的帧，[`App::apply_run_event`] 再按当前 run_id 校验一遍。
fn event_loop(
    client: &WebDaemon,
    sid: &str,
    history: Vec<ChatMessage>,
    rx: &Receiver<AgentEvent>,
) -> Result<(), TuiError> {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(std::io::stdout()))?;
    let mut app = App::new();
    app.start_session(sid, history);
    // T3：footer 的 model 段读运行时配置（一次性）；问题面板先吃一帧
    // pending 快照——订阅建立前已提交的问题不漏（route §3 消费 pending）。
    app.set_model(footer::configured_model());
    if let Ok(items) = client.pending_questions() {
        app.set_pending_questions(items);
    }
    // T3：`user.question` 推送线程 → 刷新信号；快照解码统一走 pending 路径。
    let (qtx, qrx) = mpsc::channel::<()>();
    spawn_question_sub(client, qtx)?;
    let mut last_pending_sync = Instant::now();
    // T3：stats.summary 首刷即刻做（footer 起步就有真 run 状态）。
    if let Ok(summary) = client.stats_summary(STATS_RANGE) {
        app.set_in_flight(summary.in_flight_runs);
    }
    let mut last_stats_sync = Instant::now();
    let mut panels = crate::ui::panels::load_panels(client);
    // 切台后的 run 作用域流：`Some((run_id, 接收端))`，每个 prompt 重建
    //（旧接收端丢弃即旧泵收线、旧订阅随之断开）。
    let mut run_rx: Option<(String, Receiver<AgentEvent>)> = None;
    let (ptx, prx) = mpsc::channel::<Result<(), ClientError>>();
    loop {
        // 出站：user 消息先落库（T1 同序：daemon 不自动落），切台后先
        // 订阅当前 run 再起 prompt（先订阅后 prompt，避免丢帧），最后
        // prompt 线程阻塞到 turn 结束、不占事件循环线程。
        // T12：copy 的 OSC52 写 stdout（`c` 只入队，App 无 IO）——写失败即
        // 退出：不能假装复制成功（同 prompt 落库的真值口径）。
        while let Some(text) = app.take_copy() {
            write_osc52(&text)?;
        }
        // T13：确认后的回退动作（`App` 无 IO，同 T12 [`Self::take_truncate`]
        // 的分工）——先发 `session.rewind`（快照 + 截断单事务），成功再
        // `load_messages` 重载 transcript；失败只置状态行，本地投影一个字节
        // 都不动（真值安全，不假装回退成功，route §3 注记④）。
        if let Some((from_seq, n)) = app.take_rewind() {
            let sid = app.session_id().to_string();
            match client.rewind_session(&sid, from_seq) {
                Err(err) => app.note_rewind_failed(err.to_string()),
                Ok(_snapshotted) => {
                    match crate::ui::session_picker::load_history(client, &sid) {
                        Ok(history) => app.note_rewind_ok(n, history),
                        // 回退已落库、只是回读失败：轮数报真（ledger 确实
                        // 裁了），重载失败单独明说，不静默装作全成功。
                        Err(err) => app.set_status(format!(
                            "rewound {n} turn(s) — transcript reload failed: {err}"
                        )),
                    }
                }
            }
        }
        if let Some(delivery) = app.take_prompt() {
            // T12：retry/edit 的截断必须**先于**重发文落库（否则重发的那条
            // 自己也被删）。截断点只在 idle + 空队列时挂得上（`message_key`
            // 单点 fencing），命中的必然是本次出站的那条改写。
            if let Some(from_seq) = app.take_truncate() {
                client
                    .truncate_session(&delivery.session_id, from_seq)
                    .map_err(client_error)?;
            }
            let seq = push_user_message(client, &delivery.session_id, &delivery.text)?;
            // T12：出站回执把本地投影里的这条 user 消息回填成 ledger 序号，
            // 后续 retry/edit 才截得中它（`user-{ts}` 占位 id 解析不出 seq）。
            app.note_appended_seq(seq);
            if app.run_scoped() {
                run_rx = Some((
                    delivery.run_id.clone(),
                    spawn_run_stream(client, &delivery.run_id)?,
                ));
            }
            spawn_prompt(
                client,
                &delivery.session_id,
                &delivery.run_id,
                delivery.text,
                ptx.clone(),
            )?;
        }
        terminal.draw(|frame| {
            // T6：先喂本帧几何（跟尾滑动 / clamp 在模型里推进），再渲染——
            // sync 与 draw 共用 `ui` 的同一套区域几何，窗口不漂移。
            crate::ui::sync_viewport(&mut app, frame.area());
            crate::ui::draw(frame, &app);
            crate::ui::panels::render(frame, &app, &panels);
        })?;
        if event::poll(POLL)? {
            match event::read()? {
                Event::Key(key) => {
                    // 会话切换键：空闲才开 picker（运行中忽略——排队 prompt 会跟着
                    // 搬进新会话，正是 T4 红线要防的事故）。T11：搜索 overlay
                    // 打开时也让位——overlay 短路了全部按键，Ctrl+K 不该绕过它
                    // 把 picker 叠上来（按键先落 `handle_key` 的 overlay 分派）。
                    // T13：rewind 确认态同批让位——模态盖过一切，Ctrl+K 不许绕过
                    // 确认态开 picker（按键落 `handle_key` 的模态分派）。
                    if switch_key(key) && !app.search_open() && app.rewind_confirm().is_none() {
                        if !app.is_running()
                            && let Some(picked) = crate::ui::session_picker::run_picker(client, "")?
                        {
                            let history = crate::ui::session_picker::load_history(client, &picked)?;
                            app.switch_session(&picked, history);
                            run_rx = None; // 旧 run 流随接收端丢弃（红线）
                        }
                        continue;
                    }
                    match app.handle_key(key) {
                        KeyAction::Quit => break,
                        KeyAction::Abort => {
                            app.set_status("aborting turn");
                            client.abort_worker().map_err(client_error)?;
                        }
                        KeyAction::None => {}
                    }
                }
                // T7：滚轮入队（route §3 T7——鼠标捕获只随 enhanced 进屏序列
                // 开，linear 路径收不到这些事件）；其余鼠标事件（移动/按键/
                // 拖拽）首版不接管（route §8：点击/拖选后置）。
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => app.wheel_tick(-1, Instant::now()),
                    MouseEventKind::ScrollDown => app.wheel_tick(1, Instant::now()),
                    _ => {}
                },
                // resize/paste/focus 不改状态。
                _ => {}
            }
        }
        // T9：斜杠命令意图（`App` 无 IO，picker 归事件循环开）：与 Ctrl+K
        // 同一落点、同一「运行中不开」口径；切台后的 run 作用域红线照旧
        //（旧 run 流随接收端丢弃）。
        while let Some(intent) = app.take_intent() {
            match intent {
                Intent::OpenSessions => {
                    if !app.is_running()
                        && let Some(picked) = crate::ui::session_picker::run_picker(client, "")?
                    {
                        let history = crate::ui::session_picker::load_history(client, &picked)?;
                        app.switch_session(&picked, history);
                        run_rx = None; // 旧 run 流随接收端丢弃（T4 红线）
                    }
                }
            }
        }
        // T11：搜索取数（`App` 无 IO，同意图队列的分工）——查询变更才发
        // `session.search`（scope = 当前会话，`FETCH_LIMIT` 上限），回执里
        // 的命中下标映射由 `App::set_search_hits` 收口；失败进状态行（真值
        // 安全，不静默不冒充无命中）。同步阻塞与 picker 同量级（服务端
        // LIKE 子串查询），不另起线程。
        if app.search_needs_fetch() {
            let query = app.search_query();
            match client.search_messages(&query, app.session_id(), search::FETCH_LIMIT) {
                Ok(rows) => app.set_search_hits(rows),
                Err(err) => app.set_search_error(err.to_string()),
            }
        }
        // T7 drain tick：50ms 轮询每次醒来冲刷滚轮队列（idle 时队列也能
        // 消化；16ms 下限按 Instant 记账，不动 POLL），推进结果落在下一帧
        // draw——与出站/事件流处理同拍，不额外醒循环。
        app.drain_wheel(Instant::now());
        // resize：下一次 draw 的 autoresize 自动重排（route §3：
        // 不崩、不写屏外）；paste/focus 不改状态。
        // 会话级事件流：切台前 = 视图事件源；切台后只做断线探测——旧 run
        // 的帧不许进新会话视图，直接丢。
        loop {
            match rx.try_recv() {
                Ok(ev) => {
                    if !app.run_scoped() {
                        absorb(client, &mut app, &ev, None, &mut panels)?;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    return Err(TuiError::DaemonUnreachable(
                        "daemon unreachable: event stream closed".to_string(),
                    ));
                }
            }
        }
        // T3：数字键产出的回答 → `user.answer(id, choice)`（route §3 答题
        // 路径）。daemon 打回（已答/不存在）不退 TUI——随后的快照刷新把
        // 仍 pending 的问题带回来，保留卡片的决定权在数据源。
        let mut answered = false;
        while let Some(request) = app.take_answer() {
            let _ = client.answer_question(
                &request.question_id,
                &QuestionAnswer::Select {
                    index: request.choice,
                },
            );
            answered = true;
        }
        // T3：面板刷新 = 推送信号 / 答后 / 兜底轮询；`pending_questions()`
        // 快照是服务端真相（同 id 幂等覆盖、答完消失都以它为准）。
        let mut pushed = false;
        while qrx.try_recv().is_ok() {
            pushed = true;
        }
        if pushed || answered || last_pending_sync.elapsed() >= PENDING_SYNC {
            last_pending_sync = Instant::now();
            if let Ok(items) = client.pending_questions() {
                app.set_pending_questions(items);
            }
        }
        // T3：footer 空闲段的 run 状态来自 `stats.summary`（半开 run 计数；
        // 只读统计，失败不致命，沿用上一帧值）。
        if last_stats_sync.elapsed() >= STATS_SYNC {
            last_stats_sync = Instant::now();
            if let Ok(summary) = client.stats_summary(STATS_RANGE) {
                app.set_in_flight(summary.in_flight_runs);
            }
        }
        // run 作用域流（切台后）：帧按订阅时的 run_id 校验后进视图。
        if let Some((run_id, srx)) = &run_rx {
            while let Ok(ev) = srx.try_recv() {
                absorb(client, &mut app, &ev, Some(run_id.as_str()), &mut panels)?;
            }
        }
        // prompt RPC 结果：失败即退出（错误映射同 T1：Connect→3，余→1）。
        while let Ok(res) = prx.try_recv() {
            res.map_err(client_error)?;
        }
    }
    Ok(())
}

/// T3：订阅 `user.question` 推送，帧到达即给事件循环发刷新信号。
///
/// 帧体是序列化 `QuestionItem`，但解码统一走 `pending_questions()` 快照
/// 路径（同 web `question_event_loop` 的快照兜底思路）：推送只当「有新题」
/// 的即时信号，面板数据永远取最新快照——同 id 幂等覆盖、答完消失因此
/// 天然成立。订阅失败不致命（老 daemon / 临时断连）：事件循环的兜底轮询
/// 照样刷新面板。
fn spawn_question_sub(client: &WebDaemon, tx: mpsc::Sender<()>) -> Result<(), TuiError> {
    let Ok(mut sub) = client.subscribe_user_questions() else {
        return Ok(());
    };
    std::thread::Builder::new()
        .name("oi-tui-questions".into())
        .spawn(move || {
            loop {
                match sub.next_event(crate::pump::KEEPALIVE) {
                    // 有新题：叫醒事件循环去刷快照。
                    Ok(Some(_)) => {
                        if tx.send(()).is_err() {
                            break; // 接收端已释放：收线，订阅随 Drop 断开
                        }
                    }
                    Ok(None) => {}   // keepalive tick：不退出
                    Err(_) => break, // 读错误 = 断线：轮询快照继续撑面板
                }
            }
        })
        .map_err(TuiError::Io)?;
    Ok(())
}

/// 一条事件进视图 + turn 收尾（assistant 落库 → 清在飞 run → 刷新任务面板）。
///
/// `run_id`：`Some` = 切台后的 run 作用域流（按 run_id 校准入），`None`
/// = 切台前的会话级 `rx`（T2 语义，直接投影）。
fn absorb(
    client: &WebDaemon,
    app: &mut App,
    ev: &AgentEvent,
    run_id: Option<&str>,
    panels: &mut crate::ui::panels::PanelSnapshot,
) -> Result<(), TuiError> {
    let turn_end = matches!(ev, AgentEvent::TurnEnd { .. });
    match run_id {
        Some(run_id) => app.apply_run_event(run_id, ev),
        None => app.apply_event(ev),
    }
    if turn_end {
        persist_assistant(client, app.session_id(), app.ui_state())?;
        app.note_turn_end();
        *panels = crate::ui::panels::load_panels(client);
    }
    Ok(())
}

/// 会话切换键 Ctrl+K（对齐 web quick-switcher 的 ⌘K）；只认 Press/Repeat。
fn switch_key(key: KeyEvent) -> bool {
    matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('k'))
}

/// 切台后的 run 作用域事件流：先订阅（`subscribe_worker_run`，源头按
/// run_id 丢掉旧 run 的帧）后 prompt；泵线程语义与 `crate::pump` 同一套
///（keepalive tick 不退出、读错误 = 断线收线）。
fn spawn_run_stream(client: &WebDaemon, run_id: &str) -> Result<Receiver<AgentEvent>, TuiError> {
    let sub = client.subscribe_worker_run(run_id).map_err(client_error)?;
    crate::pump::spawn(sub).map_err(TuiError::Io)
}

/// 起 prompt 线程（run_id 由出站路由生成：`r-<epoch>` 格式同 T1/CLI
/// `session resume`，进程内单调保证不与旧 run 撞号）。
fn spawn_prompt(
    client: &WebDaemon,
    sid: &str,
    run_id: &str,
    msg: String,
    tx: mpsc::Sender<Result<(), ClientError>>,
) -> Result<(), TuiError> {
    let client = client.clone();
    let sid = sid.to_string();
    let run_id = run_id.to_string();
    std::thread::Builder::new()
        .name("oi-tui-prompt".into())
        .spawn(move || {
            let res = client
                .worker_prompt_run(&sid, &run_id, &msg, &[])
                .map(|_| ());
            let _ = tx.send(res);
        })?;
    Ok(())
}
