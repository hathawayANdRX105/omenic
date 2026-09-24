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

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use omenic_web_client::ClientError;
use omenic_web_client::daemon::WebDaemon;
use omenic_web_client::{QuestionAnswer, QuestionItem};
use omenic_web_state::types::{ChatMessage, MessagePart, format_duration_ms, now_epoch_ms};
use omenic_web_state::ui_state::{AgentEvent, UiState};

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

    /// 本轮收尾：回空闲、清活动覆写与耗时起点；队首 prompt 随即具备出站
    /// 资格。
    pub fn note_turn_end(&mut self) {
        self.running = false;
        self.status.clear();
        self.run_started = None;
    }

    /// 出站队列头（空闲才出队）。出队即置 running、起耗时计时、把 user
    /// 消息折进 transcript 投影——事件循环随后落库 + 起 prompt 线程。
    pub fn next_to_send(&mut self) -> Option<String> {
        if self.running {
            return None;
        }
        let msg = self.outgoing.pop_front()?;
        self.running = true;
        self.status.clear();
        self.run_started = Some(Instant::now());
        self.ui.push_message(user_message(&msg));
        Some(msg)
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
    }
}

/// enhanced 全屏外壳（route §3 签名，不许改）。
///
/// 行为契约：进 alternate screen + raw mode（[`TermGuard`]）；布局 = 上方
/// transcript + 底部 dock（composer/活动/排队/按键提示）；Enter 非空提交
/// （`worker_prompt_run`）、空串不提交；↑/↓ 本地历史；Esc/Ctrl+C 运行中
/// `worker.abort`、空闲确认退出；Ctrl+D 空 composer 退出；daemon 断线单行
/// 错误退出码 3；**所有退出路径**都经 [`TermGuard::leave`] 还原，panic 走
/// 先装的 hook（先还原再打印 + `$TMPDIR` 崩溃报告）。
///
/// `rx` = 调用方起好的会话级 worker 事件流；`opts` 解析会话（`--session`
/// 不存在 → 退出码 2，先于进屏，错误不落在 alternate screen 里）。
pub fn run_enhanced(
    opts: TuiOptions,
    client: WebDaemon,
    rx: Receiver<AgentEvent>,
) -> Result<(), TuiError> {
    let sid = resolve_session(&client, &opts)?;
    // panic hook 先于一切终端改动：随后任何 panic 都先还原再打印。
    crate::termguard::install_panic_hook();
    let mut guard = TermGuard::new(CrosstermOps);
    if let Err(e) = guard.enter() {
        // 进入可能只成功一半：留下的半截状态要收掉。
        let _ = guard.leave();
        return Err(TuiError::Io(e));
    }
    let outcome = event_loop(&client, &sid, &rx);
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
fn event_loop(client: &WebDaemon, sid: &str, rx: &Receiver<AgentEvent>) -> Result<(), TuiError> {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(std::io::stdout()))?;
    let mut app = App::new();
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
    let (ptx, prx) = mpsc::channel::<Result<(), ClientError>>();
    loop {
        // 出站：user 消息先落库（T1 同序：daemon 不自动落），再起 prompt
        // 线程——worker_prompt_run 阻塞到 turn 结束，不占事件循环线程。
        if let Some(msg) = app.next_to_send() {
            push_user_message(client, sid, &msg)?;
            spawn_prompt(client, sid, msg, ptx.clone())?;
        }
        terminal.draw(|frame| crate::ui::draw(frame, &app))?;
        if event::poll(POLL)?
            && let Event::Key(key) = event::read()?
        {
            match app.handle_key(key) {
                KeyAction::Quit => break,
                KeyAction::Abort => {
                    app.set_status("aborting turn");
                    client.abort_worker().map_err(client_error)?;
                }
                KeyAction::None => {}
            }
        }
        // resize：下一次 draw 的 autoresize 自动重排（route §3：
        // 不崩、不写屏外）；paste/focus 不改状态。
        // 事件流：泵线程收线 = 订阅断线 = daemon 断线 → 退出码 3。
        loop {
            match rx.try_recv() {
                Ok(ev) => {
                    let turn_end = matches!(ev, AgentEvent::TurnEnd { .. });
                    app.apply_event(&ev);
                    if turn_end {
                        persist_assistant(client, sid, app.ui_state())?;
                        app.note_turn_end();
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

/// 起 prompt 线程（`r-<epoch>` run 归属，格式同 T1/CLI `session resume`）。
fn spawn_prompt(
    client: &WebDaemon,
    sid: &str,
    msg: String,
    tx: mpsc::Sender<Result<(), ClientError>>,
) -> Result<(), TuiError> {
    let run_id = format!("r-{}", now_epoch_ms());
    let client = client.clone();
    let sid = sid.to_string();
    std::thread::Builder::new()
        .name("oi-tui-prompt".into())
        .spawn(move || {
            let res = client.worker_prompt_run(&sid, &run_id, &msg).map(|_| ());
            let _ = tx.send(res);
        })?;
    Ok(())
}
