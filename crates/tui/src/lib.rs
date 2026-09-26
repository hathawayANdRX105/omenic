//! omenic-tui — `oi tui` 的终端前端（T1：linear 基线；T2：enhanced 全屏外壳）。
//!
//! 纯 daemon 客户端（route §2 铁律）：探针裁决模式（[`mode`] / [`probe`]），
//! 经 `web-client` 的 [`WebDaemon`] 订阅 worker 事件，独立泵线程进
//! mpsc（[`pump`]）→ [`UiState::apply`] 投影 → 两个渲染器共用同一投影：
//! linear 裸写 stdout（零 ESC 字节），enhanced 走 ratatui 全屏
//! （[`app::run_enhanced`]，termguard 进出 + theme 样式 + dock 按键）。
//!
//! 只依赖 `web-client` + `web-state`：路由面走 client 门面，
//! 不 import daemon 协议层；wire 帧统一过 `WireTranslator`。
//!
//! 错误出口（CLI dispatch 按变体映射退出码，route §5 smoke）：session 不存在
//! → 2，daemon 不可达/断线 → 3，其余 → 1；一律单行 `omenic tui: {err}` 到
//! stderr，不 panic、不留半还原终端（enhanced 的还原由 termguard 兜底）。

mod linear;
mod mode;
mod probe;
mod pump;

pub mod app;
pub mod scroll;
pub mod termguard;
pub mod theme;
pub mod ui;

pub use linear::render_linear_line;
pub use mode::{TuiMode, enhanced_eligible, resolve_mode};
pub use probe::{MuxKind, TermProbe};

use std::io::{BufRead, Write};
use std::sync::mpsc::{Receiver, RecvTimeoutError};

use web_client::ClientError;
use web_client::daemon::WebDaemon;
use web_state::convert::WireTranslator;
use web_state::types::now_epoch_ms;
use web_state::ui_state::{AgentEvent, UiState};

/// `oi tui` 运行选项（CLI 解析后传入；route §3 契约字段，不许改）。
#[derive(Debug, Clone)]
pub struct TuiOptions {
    /// 请求档位（auto/enhanced/linear）；裁决见 [`resolve_mode`]。
    pub mode: TuiMode,
    /// 指定已有会话 id；不存在 → [`TuiError::SessionNotFound`]（退出码 2）。
    pub session: Option<String>,
    /// 续接最近活跃会话；没有会话 → 同上。
    pub resume: bool,
    /// 禁用颜色（同 `NO_COLOR`）：探针 color 置 false 后再裁决。
    pub no_color: bool,
    /// 抑制 enhanced 外壳的非必要动效（route §3 `--reduced-motion` 接线）。
    pub reduced_motion: bool,
}

/// `oi tui` 运行期错误（thiserror 单行 Display；CLI 按变体映射退出码）。
#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    /// 指定/续接的会话不存在（内容即完整单行文案；CLI → 退出码 2）。
    #[error("{0}")]
    SessionNotFound(String),
    /// daemon 连不上 / 轮次中断线（内容即完整单行文案；CLI → 退出码 3）。
    #[error("{0}")]
    DaemonUnreachable(String),
    /// daemon 返回结构化错误 / 协议错（CLI → 退出码 1；文案自带 daemon 前缀）。
    #[error("{0}")]
    Daemon(String),
    /// 本地 IO（stdout 写、stdin 读）。
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// 跑 `oi tui`（route §3 签名，不许改）：探针 → 裁决 → 分派渲染器——
/// enhanced 走 [`app::run_enhanced`] 全屏外壳，linear 走「解析会话 →
/// 读行 → 落库 → 订阅 → prompt → 投影到 TurnEnd」循环，EOF 干净退出 0。
pub fn run(opts: TuiOptions) -> Result<(), TuiError> {
    // 1) 探针：`--no-color` / `NO_COLOR` 都折进 color，resolve 只看快照。
    let mut probe = TermProbe::from_env();
    if opts.no_color {
        probe.color = false;
    }
    // 2) 裁决：门全过 → enhanced 全屏，否则 linear（route §3）。
    let mode = resolve_mode(opts.mode, &probe);
    // 3) daemon 门面：连 socket 都没有 = 没起 daemon → 退出码 3，不 panic。
    let daemon = WebDaemon::from_env_or_default().ok_or_else(|| {
        TuiError::DaemonUnreachable(
            "daemon unreachable: daemon socket not found (try `oi daemon start`)".to_string(),
        )
    })?;
    // 4) enhanced：一次会话级订阅喂全程（多次 prompt 的事件同流进，
    //    断线 = 泵收线 = 退出码 3），分派全屏外壳。
    if let TuiMode::Enhanced = mode {
        let rx = spawn_worker_stream(&daemon)?;
        return app::run_enhanced(opts, daemon, rx);
    }
    // 5) linear（T1 基线不动）：解析会话 → 交互循环。
    let sid = resolve_session(&daemon, &opts)?;
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut line = String::new();
    loop {
        print!("> ");
        std::io::stdout().flush()?;
        line.clear();
        if input.read_line(&mut line)? == 0 {
            return Ok(()); // EOF（含管道喂完）：干净退出，退出码 0
        }
        let msg = line.trim_end_matches(['\n', '\r']).to_string();
        if msg.trim().is_empty() {
            continue;
        }
        // user 消息先落库（daemon 不自动落，page-workspace 同序）：resume
        // 上下文回放（dispatch 的 WorkerPrompt 臂）读的就是这张表。
        push_user_message(&daemon, &sid, &msg)?;
        let run_id = format!("r-{}", now_epoch_ms());
        // 先订阅后 prompt（route §3 边界）：prompt 返回即可能开跑，事件一帧
        // 都不能漏——语义见 daemon dispatch.rs 的 G7-B set_active_run 注释。
        let rx = pump::spawn(daemon.subscribe_worker_run(&run_id).map_err(client_error)?)?;
        daemon
            .worker_prompt_run(&sid, &run_id, &msg, &[])
            .map_err(client_error)?;
        // 6) 投影本轮：每事件 linear 落屏 + flush，TurnEnd 收行后落库回复。
        let mut state = UiState::default();
        render_until_turn_end(&mut state, &rx)?;
        persist_assistant(&daemon, &sid, &state)?;
    }
}

/// 起会话级 worker 事件泵（enhanced 用）：一条 `subscribe_worker` 长连接
/// → 独立线程读帧过 [`WireTranslator`] → `mpsc<AgentEvent>`。与 [`pump`]
/// 同一套 keepalive/断线语义；差别只在这条流不按 run 过滤——会话全程只有
/// 一条，多次 prompt 的事件都从这里进，泵线程收线即调用方的断线信号。
fn spawn_worker_stream(client: &WebDaemon) -> Result<Receiver<AgentEvent>, TuiError> {
    let mut sub = client.subscribe_worker().map_err(client_error)?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("oi-tui-pump".into())
        .spawn(move || {
            let mut translator = WireTranslator::new();
            loop {
                match sub.next_event(pump::KEEPALIVE) {
                    Ok(Some(frame)) => {
                        if let Some(ev) = translator.translate(&frame.event)
                            && tx.send(ev).is_err()
                        {
                            break; // 接收端已释放：收线，订阅随 Drop 断开
                        }
                    }
                    Ok(None) => {}   // keepalive tick：不退出
                    Err(_) => break, // 读错误 = 断线：关 channel 就是信号
                }
            }
        })
        .map_err(TuiError::Io)?;
    Ok(rx)
}

/// 一条订阅投影到 `TurnEnd`：每事件 [`render_linear_line`] + flush；
/// keepalive 超时继续等（route §3：tick 不退出）；泵线程退出（订阅断线）
/// → 退出码 3 的单行错误。
fn render_until_turn_end(state: &mut UiState, rx: &Receiver<AgentEvent>) -> Result<(), TuiError> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    loop {
        match rx.recv_timeout(pump::KEEPALIVE) {
            Ok(ev) => {
                let turn_end = matches!(ev, AgentEvent::TurnEnd { .. });
                render_linear_line(&ev, state, &mut out)?;
                out.flush()?;
                if turn_end {
                    return Ok(());
                }
            }
            // keepalive tick：订阅还活着，继续等下一帧。
            Err(RecvTimeoutError::Timeout) => continue,
            // 泵线程退出 = 订阅断线（daemon 被 kill 等）。
            Err(RecvTimeoutError::Disconnected) => {
                return Err(TuiError::DaemonUnreachable(
                    "daemon unreachable: event stream closed".to_string(),
                ));
            }
        }
    }
}

/// 解析会话 id：`--session`（须在列表内，否则退出码 2）→ `--resume`
/// （最近活跃一条，空列表同报错）→ 新建 `s-{epoch}`。
///
/// 存在性用 `list_sessions(1000)` 成员判定：`WebDaemon` 没有暴露
/// `session_get` 直查（client 的 search_sessions 内部才有），全量列表即判据；
/// `list_sessions` 存储侧按最近活跃排序，首条即「最近」。
fn resolve_session(daemon: &WebDaemon, opts: &TuiOptions) -> Result<String, TuiError> {
    if let Some(sid) = &opts.session {
        let sessions = daemon.list_sessions(1000).map_err(client_error)?;
        if sessions.iter().any(|s| s.id == *sid) {
            return Ok(sid.clone());
        }
        return Err(TuiError::SessionNotFound(format!(
            "session not found: {sid}"
        )));
    }
    if opts.resume {
        let mut recent = daemon.list_sessions(1).map_err(client_error)?;
        if let Some(s) = recent.pop() {
            return Ok(s.id);
        }
        return Err(TuiError::SessionNotFound(
            "no session to resume: session list is empty".to_string(),
        ));
    }
    // 新建会话（create 幂等：撞 id 原样返回原行）。标题用 epoch 占位，
    // 首条消息落库后可由 web 侧按 title_from_first_message 派生真标题。
    let ms = now_epoch_ms();
    let sid = format!("s-{ms}");
    daemon
        .create_session(&sid, &format!("会话 {ms}"))
        .map_err(client_error)?;
    Ok(sid)
}

/// user 消息落库（`role_user = true`），必须发生在 prompt 之前。
fn push_user_message(daemon: &WebDaemon, sid: &str, text: &str) -> Result<(), TuiError> {
    daemon
        .append_message(sid, true, text, &[])
        .map_err(client_error)?;
    Ok(())
}

/// 本轮 assistant 正文落库（`role_user = false`）。回复同样不自动落库
/// （page-workspace 在 turn 结束后自己 append，见其 worker_event_loop），
/// 不补这一笔则 `session attach` 只看得到提问、resume 回放读不到上一轮
/// 回答。空回复不落。
fn persist_assistant(daemon: &WebDaemon, sid: &str, state: &UiState) -> Result<(), TuiError> {
    let text: String = last_assistant_content(state);
    if text.is_empty() {
        return Ok(());
    }
    daemon
        .append_message(sid, false, &text, &[])
        .map_err(client_error)?;
    Ok(())
}

/// 最近一条 assistant 的正文（每轮 state 是本轮新建的，即本轮回复）。
fn last_assistant_content(state: &UiState) -> String {
    state
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "assistant")
        .map(|m| m.content.clone())
        .unwrap_or_default()
}

/// daemon 通信错误 → TuiError：连接失败（Connect）→ 退出码 3 的
/// unreachable；服务端结构化错误/协议/编码错 → 退出码 1 的 daemon 错。
/// 后三类直接借 `ClientError` 的单行 Display（`daemon error …` 等）。
fn client_error(e: ClientError) -> TuiError {
    match e {
        ClientError::Connect(io) => {
            TuiError::DaemonUnreachable(format!("daemon unreachable: {io}"))
        }
        other => TuiError::Daemon(other.to_string()),
    }
}
