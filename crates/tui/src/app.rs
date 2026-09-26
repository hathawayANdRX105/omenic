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
    /// 出站队列：空闲时头部即刻发，运行中整队等本轮 `TurnEnd`。
    outgoing: VecDeque<String>,
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
}

impl App {
    /// 空会话状态（事件循环与测试的同一入口）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 喂一个字符按键（测试与事件循环共用的打字辅助）。
    pub fn type_char(&mut self, c: char) -> KeyAction {
        self.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    /// composer 当前行。
    pub fn input(&self) -> &str {
        &self.input
    }

    /// 本轮是否在跑。
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// 退出确认是否未决（dock 提示行消费）。
    pub fn confirm_quit(&self) -> bool {
        self.confirm_quit
    }

    /// 队首待发 prompt（运行中 = 排队；dock 排队行消费）。
    pub fn queued(&self) -> Option<&str> {
        self.outgoing.front().map(String::as_str)
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
        self.status.clear();
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
        self.running = true;
        self.status.clear();
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
        self.running = false;
        self.current_run = None;
        self.status.clear();
        self.confirm_quit = false;
        self.scroll = 0;
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
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            // 退出/中断双键与确认态：先于一切编辑路由。
            KeyCode::Esc => return self.on_escape(),
            KeyCode::Char('c') if ctrl => return self.on_escape(),
            // Ctrl+D 只在 composer 为空时是退出（非空 = 忽略，不吞输入）。
            KeyCode::Char('d') if ctrl && self.input.is_empty() => return KeyAction::Quit,
            KeyCode::Char('d') if ctrl => return KeyAction::None,
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
        match key.code {
            KeyCode::Enter => self.submit_line(),
            // T3：一个键全部展开/折叠工具卡（route §3；Tab 不进 composer）。
            KeyCode::Tab => self.tools_expanded = !self.tools_expanded,
            KeyCode::Backspace => {
                self.input.pop();
                self.history_pos = None;
            }
            KeyCode::Up => self.history_back(),
            KeyCode::Down => self.history_forward(),
            // T6：transcript 视口键（route §3 T6）。PgUp/PgDn/End 与 composer
            // 无关——单行输入没有翻页/到头语义（光标恒在行尾），不吞编辑；
            // Ctrl+U 半页只在 composer 为空时归滚动，有文本落回编辑语义
            // （现有编辑不认 Ctrl+U = 忽略不吞字，jcode 仲裁口径）。↑↓ 历史
            // 行为不动（contract §3 键位仲裁第一条）。
            KeyCode::PageUp => self.viewport.page_up(),
            KeyCode::PageDown => self.viewport.page_down(),
            KeyCode::End => self.viewport.to_end(),
            KeyCode::Char('u') if ctrl && self.input.is_empty() => self.viewport.half_up(),
            KeyCode::Char(c) if !ctrl => {
                self.input.push(c);
                self.history_pos = None;
            }
            _ => {}
        }
        KeyAction::None
    }

    /// Esc / Ctrl+C：运行中 = Abort；空闲 = 第一下要确认、第二下才 Quit。
    fn on_escape(&mut self) -> KeyAction {
        if self.running {
            return KeyAction::Abort;
        }
        if self.confirm_quit {
            return KeyAction::Quit;
        }
        self.confirm_quit = true;
        KeyAction::None
    }

    /// Enter：非空（trim 后）进历史 + 出站队列；空/纯空白只清行不提交
    /// （route §3：Enter 空串不提交——空行误发由这里挡）。
    fn submit_line(&mut self) {
        let text = std::mem::take(&mut self.input);
        self.history_pos = None;
        if text.trim().is_empty() {
            return;
        }
        self.history.push(text.clone());
        self.outgoing.push_back(text);
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
        if let Some(delivery) = app.take_prompt() {
            push_user_message(client, &delivery.session_id, &delivery.text)?;
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
                    // 搬进新会话，正是 T4 红线要防的事故）。
                    if switch_key(key) {
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
