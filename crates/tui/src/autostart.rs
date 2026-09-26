//! T14 daemon 自启动门：探活 → spawn → wait-ready（route §3 T14）。
//!
//! `oi tui` 进渲染循环之前必须确认 daemon 可答——已知坑「冷启动首条
//! prompt 静默丢弃」的规避手段是**就绪判定只用 `daemon.ping` 往返**，绝不
//! 发假 prompt 试探。三阶段失败各自带阶段与原因返回，由 CLI 侧
//! `eprintln` 呈现并映射既有退出码（不静默、不假死等 daemon）。
//!
//! 核心 [`ensure_ready`] 的探活与 spawn 是注入闭包（可换桩，测试不真起
//! 进程）；生产接线 [`ensure_daemon_running`] 才绑 `WebDaemon::ping` 与
//! `std::process::Command`。daemon 二进制路径由调用方（`oi tui` 分发臂）
//! 按既有 sibling / `OMENIC_DAEMON_PATH` 规则解析后传入——路径解析的单
//! 一出处在 CLI 侧，本模块只消费解析结果（D18 同款解析禁止到处重写）。

use std::path::Path;
use std::time::{Duration, Instant};

use web_client::daemon::WebDaemon;

/// 探活节拍：对齐 `oi daemon start` 的既有轮询（20ms 一拍）。
const PING_INTERVAL: Duration = Duration::from_millis(20);

/// wait-ready 总预算：对齐 `oi daemon start` 的 5s（250 × 20ms）。
const READY_TIMEOUT: Duration = Duration::from_secs(5);

/// 启动门结果。`AlreadyRunning` = 探活发现 daemon 已活（零 spawn 直接放
/// 行）；`Ready` = spawn 之后 ping 转活（wait-ready 完成后才可能拿到）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartOutcome {
    /// 探活已活：不 spawn，直接进 TUI。
    AlreadyRunning,
    /// 冷启动 spawn 后等到 ping 可答：此刻交接，首条 prompt 不会落空。
    Ready,
}

/// 失败阶段：探活 / spawn / wait-ready 三段各归各，错误带上它，「哪一步
/// 挂的」一眼可判。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutostartStage {
    /// 探活本身不可用（区别于「daemon 不在」的 `Ok(false)`）。
    Probe,
    /// 子进程拉不起来（路径坏、权限、fork 失败……）。
    Spawn,
    /// spawn 后在总预算内没等到 ping 可答。
    WaitReady,
}

impl std::fmt::Display for AutostartStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AutostartStage::Probe => f.write_str("probe"),
            AutostartStage::Spawn => f.write_str("spawn"),
            AutostartStage::WaitReady => f.write_str("wait-ready"),
        }
    }
}

/// 启动门失败：阶段 + 原因，单行 Display（CLI 侧 `omenic tui: {err}` 直打）。
#[derive(Debug, thiserror::Error)]
#[error("daemon autostart {stage} failed: {reason}")]
pub struct AutostartError {
    /// 失败发生在哪一阶段。
    pub stage: AutostartStage,
    /// 该阶段的具体原因（保持单行，不带换行）。
    pub reason: String,
}

impl AutostartError {
    fn new(stage: AutostartStage, reason: impl Into<String>) -> Self {
        AutostartError {
            stage,
            reason: reason.into(),
        }
    }
}

/// 探活 / spawn / wait-ready 三阶段启动门（核心逻辑，依赖注入）。
///
/// - `probe`：一次探活，`Ok(true)` = daemon 可答、`Ok(false)` = 不在、
///   `Err` = 探活自身不可用（探活阶段归 [`AutostartStage::Probe`]，
///   wait-ready 阶段的探活失败归 [`AutostartStage::WaitReady`]）；
/// - `spawn`：拉起 `daemon_bin` 指向的二进制，失败原因原样进错误；
/// - `ping_interval` / `ready_timeout`：wait-ready 的节拍与总预算。
///
/// 返回 `AlreadyRunning` 时 spawn 恰好零次；返回 `Ready` 之前一定有一次
/// `Ok(true)` 的探活（T14：未 Ready 不放行，冷启动首条 prompt 不丢）。
pub fn ensure_ready<P, S>(
    daemon_bin: &Path,
    ping_interval: Duration,
    ready_timeout: Duration,
    mut probe: P,
    mut spawn: S,
) -> Result<StartOutcome, AutostartError>
where
    P: FnMut() -> Result<bool, String>,
    S: FnMut(&Path) -> Result<(), String>,
{
    // 探活已活 → 零 spawn 直接放行（并发两路 `oi tui` 的后到者走这条）。
    if probe_once(&mut probe)? {
        return Ok(StartOutcome::AlreadyRunning);
    }
    // spawn 前复查：首探与 spawn 之间可能有另一路刚把 daemon 拉起来，再
    // ping 一次收窄「双 spawn」窗口；真撞上了也由子进程的实例锁收敛（见
    // `spawn_daemon` 注释），这里的复查只是把常见竞态挡在 spawn 之外。
    if probe_once(&mut probe)? {
        return Ok(StartOutcome::AlreadyRunning);
    }

    spawn(daemon_bin).map_err(|reason| AutostartError::new(AutostartStage::Spawn, reason))?;

    // wait-ready：短间隔 ping 到总预算为止。就绪判定只认 ping 往返——
    // 假 prompt 会把「冷启动首条 prompt 静默丢弃」的坑原样引进来。
    let deadline = Instant::now() + ready_timeout;
    loop {
        let alive = probe().map_err(|reason| {
            AutostartError::new(
                AutostartStage::WaitReady,
                format!("probe failed while waiting: {reason}"),
            )
        })?;
        if alive {
            return Ok(StartOutcome::Ready);
        }
        if Instant::now() >= deadline {
            return Err(AutostartError::new(
                AutostartStage::WaitReady,
                format!(
                    "daemon did not answer ping within {}ms after spawning {}",
                    ready_timeout.as_millis(),
                    daemon_bin.display()
                ),
            ));
        }
        std::thread::sleep(ping_interval);
    }
}

/// 一次探活；`Err` 归探活阶段（与 spawn 后 wait 阶段的探活失败分开归属）。
fn probe_once<P>(probe: &mut P) -> Result<bool, AutostartError>
where
    P: FnMut() -> Result<bool, String>,
{
    probe().map_err(|reason| AutostartError::new(AutostartStage::Probe, reason))
}

/// 生产接线：真实探活（socket 存在性 + `daemon.ping` 往返，走
/// `WebDaemon`，协议 `daemon.ping` 已有、零 RPC 增补）+ 真实 spawn。
/// 节拍与总预算取本模块常量，与 `oi daemon start` 的轮询同口径。
pub fn ensure_daemon_running(daemon_bin: &Path) -> Result<StartOutcome, AutostartError> {
    ensure_ready(
        daemon_bin,
        PING_INTERVAL,
        READY_TIMEOUT,
        || Ok(WebDaemon::from_env_or_default().is_some_and(|daemon| daemon.ping())),
        spawn_daemon,
    )
}

/// 拉起 daemon 子进程，形态对齐 `oi daemon start` 的既有 spawn：无参数、
/// stdio 全 null（daemon 的启动文案不能打进 TUI 屏幕）、不等子进程返回
/// （判活交给 wait-ready 的 ping 轮询）。
///
/// 防重语义在 daemon 二进制自己手里：它起步先抢 `<socket>.lock` 实例锁，
/// 抢不到 = 已有实例在跑 → 立即非零退出（`DaemonError::AlreadyRunning`）。
/// 两路并发 spawn 因此收敛到同一个 daemon，不会出现双实例。
fn spawn_daemon(bin: &Path) -> Result<(), String> {
    let mut child = std::process::Command::new(bin)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to spawn {}: {e}", bin.display()))?;
    // `oi tui` 比这次 spawn 活得长：不回收的话 daemon 将来退出会留一个
    // 僵尸进程挂在 TUI 名下。收尸线程跟到子进程退出为止（子进程长期在跑
    // 时线程只是阻塞等待，不占 CPU）。
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
