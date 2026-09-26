//! daemon_autostart — route §3 T14 / issue #499 契约：探活已活零 spawn、
//! 冷启动 spawn 后 wait-ready 到 ping 可答才放行（冷启动首条 prompt 不丢
//! 的库层钉）、三阶段失败带阶段与原因、并发竞态不重复 spawn。
//!
//! 全程走 [`ensure_ready`] 的注入桩（探活 / spawn 闭包可换）：无 IO、不真
//! 起进程，沿 `tests/keys.rs` / `queue_fifo.rs` 的 App 无 IO 范式。
//!
//! 「首 prompt 不丢」的挂靠点说明：既有出站序列测试（`queue_fifo.rs` /
//! `keys.rs` 的 `next_to_send`）断言的是 App 层出站次序，不含 daemon 就绪
//! 条件——按任务书 fallback 在库函数层钉「未 Ready 不放行」。

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use omenic_tui::autostart::{AutostartError, AutostartStage, StartOutcome, ensure_ready};

/// 探活节拍与总超时：桩不真等 daemon，压到毫秒级（核心逻辑把两者当参数
/// 收，常量只属于生产接线）。超时留 200ms 是给 CI 满载时的调度抖动余量
/// ——期望 Ready 的用例必须跑在预算内，期望超时的用例只多等这 200ms。
const TICK: Duration = Duration::from_millis(1);
const TIMEOUT: Duration = Duration::from_millis(200);

/// 传给 `ensure_ready` 的假路径：spawn 是桩，路径只进事件账本与错误文案。
const BIN: &str = "/omenic/daemon";

/// 共享桩：探活脚本 + spawn 结果 + 事件账本。事件按发生顺序落账，时序类
/// 断言（Ready 前不返回、spawn 只发生一次）都读这一本账。
struct Stub {
    /// 每次探活弹出一条；弹空后一直返回 `tail`。
    script: VecDeque<Result<bool, String>>,
    /// 脚本耗尽后的探活答案。
    tail: Result<bool, String>,
    /// spawn 桩失败原因（Some = spawn 报错）。
    spawn_error: Option<String>,
    /// spawn 成功后把探活翻成「已活」（spawn 期间另一路拉起 daemon 的竞态桩）。
    revive_on_spawn: bool,
    probes: usize,
    spawns: usize,
    events: Vec<String>,
}

impl Stub {
    /// 缺省「daemon 不在」：冷启动类测试的起点。
    fn dead() -> Self {
        Stub {
            script: VecDeque::new(),
            tail: Ok(false),
            spawn_error: None,
            revive_on_spawn: false,
            probes: 0,
            spawns: 0,
            events: Vec::new(),
        }
    }

    /// 缺省「已在跑」：首次探活即活。
    fn alive() -> Self {
        let mut stub = Self::dead();
        stub.tail = Ok(true);
        stub
    }

    /// 覆盖前 N 次探活的答案（脚本耗尽回 `tail`）。
    fn script(mut self, answers: &[Result<bool, String>]) -> Self {
        self.script = answers.iter().cloned().collect();
        self
    }
}

/// 把共享桩接进 `ensure_ready` 的两个注入闭包——每个测试只此一处构造，
/// 探活与 spawn 的记账口径全文件唯一。
fn run(stub: &Rc<RefCell<Stub>>) -> Result<StartOutcome, AutostartError> {
    let probe_side = Rc::clone(stub);
    let spawn_side = Rc::clone(stub);
    ensure_ready(
        Path::new(BIN),
        TICK,
        TIMEOUT,
        move || {
            let mut stub = probe_side.borrow_mut();
            stub.probes += 1;
            let answer = stub.script.pop_front().unwrap_or_else(|| stub.tail.clone());
            stub.events.push(
                match &answer {
                    Ok(true) => "probe:alive",
                    Ok(false) => "probe:dead",
                    Err(_) => "probe:err",
                }
                .to_string(),
            );
            answer
        },
        move |bin| {
            let mut stub = spawn_side.borrow_mut();
            stub.spawns += 1;
            stub.events.push(format!("spawn:{}", bin.display()));
            if let Some(reason) = &stub.spawn_error {
                return Err(reason.clone());
            }
            if stub.revive_on_spawn {
                stub.tail = Ok(true);
            }
            Ok(())
        },
    )
}

/// 已活路径：探活一次即放行，spawn 桩零次（并发后到者的路径）。
#[test]
fn already_alive_returns_without_spawn() {
    let stub = Rc::new(RefCell::new(Stub::alive()));

    let outcome = run(&stub).expect("已活必须放行");

    let stub = stub.borrow();
    assert_eq!(outcome, StartOutcome::AlreadyRunning);
    assert_eq!(stub.spawns, 0, "已活零 spawn");
    assert_eq!(stub.events, ["probe:alive"], "首探即活，不进 spawn/wait");
}

/// 冷启动路径：探活死 → spawn 桩 → 探活转活 → Ready。账本钉住时序——
/// Ready 返回前最后一条事件必须是 `probe:alive`（提前在 spawn 后就返回
/// Ready 的实现会以 `spawn` 收尾，直接判红）。
#[test]
fn cold_start_spawns_then_waits_until_probe_answers() {
    let stub = Rc::new(RefCell::new(Stub::dead().script(&[
        Ok(false),
        Ok(false),
        Ok(false),
        Ok(true),
    ])));

    let outcome = run(&stub).expect("转活后必须 Ready");

    let stub = stub.borrow();
    assert_eq!(outcome, StartOutcome::Ready);
    assert_eq!(stub.spawns, 1, "冷启动恰好一次 spawn");
    assert_eq!(
        stub.events,
        [
            "probe:dead",
            "probe:dead",
            "spawn:/omenic/daemon",
            "probe:dead",
            "probe:alive",
        ],
        "Ready 只能发生在探活转活之后"
    );
}

/// cold-start 首 prompt 不丢（库层钉）：探活一直不转活 → wait-ready 超时
/// 报错，**绝不**返回 Ok 放行——未 Ready 不放行，交接给 TUI 的前提不成立。
#[test]
fn unready_gate_never_hands_off() {
    let stub = Rc::new(RefCell::new(Stub::dead()));

    let err = run(&stub).expect_err("未 Ready 不许返回 Ok");

    let stub = stub.borrow();
    assert_eq!(err.stage, AutostartStage::WaitReady);
    assert!(
        !stub.events.iter().any(|e| e == "probe:alive"),
        "整个等待期都没转活"
    );
    assert_eq!(stub.spawns, 1, "只 spawn 一次，超时即失败可见");
    assert!(
        err.reason.contains("did not answer ping"),
        "超时原因要可读：{}",
        err.reason
    );
}

/// 失败可见之一：spawn 桩失败 → 阶段 = spawn、原因原样带出。
#[test]
fn spawn_failure_reports_stage_and_reason() {
    let mut stub = Stub::dead();
    stub.spawn_error = Some("failed to spawn /omenic/daemon: broken executable".to_string());
    let stub = Rc::new(RefCell::new(stub));

    let err = run(&stub).expect_err("spawn 失败必须返回 Err");

    assert_eq!(err.stage, AutostartStage::Spawn);
    assert!(err.reason.contains("broken executable"), "{}", err.reason);
    assert_eq!(err.to_string().lines().count(), 1, "CLI 单行呈现");
}

/// 失败可见之二：探活自身出错（非「daemon 不在」）→ 阶段 = probe，且不进
/// spawn。
#[test]
fn probe_failure_reports_stage_and_reason() {
    let stub = Rc::new(RefCell::new(
        Stub::dead().script(&[Err("socket path unresolvable".to_string())]),
    ));

    let err = run(&stub).expect_err("探活失败必须返回 Err");

    let stub = stub.borrow();
    assert_eq!(err.stage, AutostartStage::Probe);
    assert!(
        err.reason.contains("socket path unresolvable"),
        "{}",
        err.reason
    );
    assert_eq!(stub.spawns, 0, "探活失败不 spawn");
}

/// 失败可见之三：wait-ready 超时 → 阶段 = wait-ready，原因带预算与路径。
#[test]
fn wait_timeout_reports_stage_and_reason() {
    let stub = Rc::new(RefCell::new(Stub::dead()));

    let err = run(&stub).expect_err("超时必须返回 Err");

    let stub = stub.borrow();
    assert_eq!(err.stage, AutostartStage::WaitReady);
    assert!(err.reason.contains(BIN), "{}", err.reason);
    assert!(stub.probes > 2, "等待期按节拍反复探活，不是一拍定生死");
}

/// 并发竞态：spawn 桩执行期间探活已转活（另一路先起了 daemon）→ 本路
/// 收敛到 Ready 且**不**再 spawn 第二次（重复 spawn 会被子进程实例锁挡
/// 回，但这里连第二次都不发起）。
#[test]
fn revival_during_spawn_does_not_spawn_again() {
    let mut stub = Stub::dead();
    stub.revive_on_spawn = true;
    let stub = Rc::new(RefCell::new(stub));

    let outcome = run(&stub).expect("转活后必须 Ready");

    let stub = stub.borrow();
    assert_eq!(outcome, StartOutcome::Ready);
    assert_eq!(stub.spawns, 1, "竞态下也只 spawn 一次");
    assert_eq!(stub.events.last().map(String::as_str), Some("probe:alive"));
}

/// spawn 前复查：首探报死、复查转活 → 零 spawn 直接放行（收窄双 spawn
/// 窗口的那一拍）。
#[test]
fn probe_recheck_before_spawn_skips_spawn() {
    let stub = Rc::new(RefCell::new(Stub::dead().script(&[Ok(false), Ok(true)])));

    let outcome = run(&stub).expect("复查发现已活必须放行");

    let stub = stub.borrow();
    assert_eq!(outcome, StartOutcome::AlreadyRunning);
    assert_eq!(stub.spawns, 0, "复查转活则零 spawn");
    assert_eq!(stub.probes, 2, "首探 + 复查，共两拍");
}
