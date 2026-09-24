//! WP-C：run 记录 → 会话列表状态推断的单测（Idle / Active / Aborted 三态）。
//!
//! daemon 的 `run.list` 语义冻结，web 侧只能拿 `RunRecord` 组装状态；
//! `infer_session_status` 是纯函数，这里按「无 run / 全关闭 / 半开孤儿 /
//! 在飞 run」四个输入形态覆盖判定表。

use omenic_web_state::convert::infer_session_status;
use omenic_web_state::types::SessionStatus;
use session::RunRecord;

/// 造一条 run 记录：`finished_at_ms` 为 None 即「有开始无 closer」的半开 run
/// （daemon 侧只有 run 结束时才写 finished_at_ms 与 status）。
fn run(run_id: &str, session_id: &str, finished: Option<i64>) -> RunRecord {
    RunRecord {
        seq: 1,
        run_id: run_id.to_string(),
        session_id: session_id.to_string(),
        started_at_ms: 1000,
        finished_at_ms: finished,
        status: finished.map(|_| "ok".to_string()),
    }
}

#[test]
fn no_runs_is_idle() {
    assert_eq!(infer_session_status(&[], None), SessionStatus::Idle);
}

#[test]
fn all_finished_runs_are_idle() {
    let runs = [run("r-1", "s-1", Some(2000)), run("r-2", "s-1", Some(3000))];
    assert_eq!(infer_session_status(&runs, None), SessionStatus::Idle);
}

#[test]
fn half_open_run_without_live_context_is_aborted() {
    // 有开始无 closer，页面刚刷新（无在飞上下文）→ Aborted：
    // 这是「手动中断 run 后刷新，列表标 aborted 而非消失」的入口
    let runs = [run("r-1", "s-1", None)];
    assert_eq!(infer_session_status(&runs, None), SessionStatus::Aborted);
}

#[test]
fn half_open_run_mixed_with_finished_is_aborted() {
    // 正常结束的 run 不能掩盖崩溃孤儿
    let runs = [run("r-1", "s-1", Some(2000)), run("r-2", "s-1", None)];
    assert_eq!(infer_session_status(&runs, None), SessionStatus::Aborted);
}

#[test]
fn lone_half_open_run_matching_live_id_is_active() {
    // 唯一未关闭的 run 就是当前页面在飞的那个 → Active
    let runs = [run("r-1", "s-1", None)];
    assert_eq!(
        infer_session_status(&runs, Some("r-1")),
        SessionStatus::Active
    );
}

#[test]
fn live_run_alongside_orphan_is_aborted() {
    // 在飞 run 之外还有未关闭的孤儿 run → 列表必须暴露孤儿
    let runs = [run("r-1", "s-1", None), run("r-2", "s-1", None)];
    assert_eq!(
        infer_session_status(&runs, Some("r-2")),
        SessionStatus::Aborted
    );
}

#[test]
fn live_id_not_matching_half_open_is_aborted() {
    // 未关闭的 run 与在飞 id 对不上 → 视作孤儿
    let runs = [run("r-1", "s-1", None)];
    assert_eq!(
        infer_session_status(&runs, Some("r-999")),
        SessionStatus::Aborted
    );
}

#[test]
fn finished_run_ignores_live_run_id() {
    // run 已正常关闭，即便 live_run_id 指向它也不是 Active
    let runs = [run("r-1", "s-1", Some(2000))];
    assert_eq!(
        infer_session_status(&runs, Some("r-1")),
        SessionStatus::Idle
    );
}
