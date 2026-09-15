//! WP-C / 5.6：statusline 计时与 run→任务卡投影的单测。
//!
//! 这两块是纯函数（不依赖 daemon、不需要起服），直接覆盖行为表：
//! - [`format_duration_ms`] 的四档边界与跨界点；
//! - [`StatusLine`] 的 start/finish 计时不变式，特别是**重复结算不清零**
//!   （上一轮已结束的耗时不能被一次无在飞 run 的 `finish_run` 抹掉）；
//! - [`TaskItem::from_run`] / [`run_task_status`] 的状态映射与「不编造」
//!   约定（run 记录没有验收标准，acceptance 必须留空）。
//!
//! 计时一律用显式传入时刻的 `elapsed_label_at`，不依赖系统时钟，保证
//! 并行测试线程互不影响。

use omenic_web_state::types::{
    DEFAULT_CONTEXT_MAX, StatusLine, TaskItem, format_duration_ms, run_task_status,
};

// ── format_duration_ms ───────────────────────────────────────────────────────

#[test]
fn duration_under_a_second_shows_raw_milliseconds() {
    assert_eq!(format_duration_ms(0), "0ms");
    assert_eq!(format_duration_ms(1), "1ms");
    assert_eq!(format_duration_ms(999), "999ms");
}

#[test]
fn duration_crosses_into_seconds_at_exactly_1000ms() {
    // 跨界点：999ms 还是 ms 档，1000ms 进入秒档
    assert_eq!(format_duration_ms(999), "999ms");
    assert_eq!(format_duration_ms(1000), "1.0s");
    assert_eq!(format_duration_ms(1500), "1.5s");
    assert_eq!(format_duration_ms(59_999), "60.0s");
}

#[test]
fn duration_under_an_hour_shows_minutes_and_seconds() {
    assert_eq!(format_duration_ms(60_000), "1m00s");
    assert_eq!(format_duration_ms(72_000), "1m12s");
    // 不足一小时的边界按截断显示，不进位：59m59s 不会变成 60m00s
    assert_eq!(format_duration_ms(3_599_999), "59m59s");
}

#[test]
fn duration_at_and_above_an_hour_shows_hours_and_minutes() {
    assert_eq!(format_duration_ms(3_600_000), "1h00m");
    assert_eq!(format_duration_ms(3_780_000), "1h03m");
    assert_eq!(format_duration_ms(86_400_000), "24h00m");
}

// ── StatusLine timing ────────────────────────────────────────────────────────

#[test]
fn empty_status_line_has_neutral_fields_and_no_timing() {
    let st = StatusLine::empty();
    assert_eq!(st.model, "");
    assert_eq!(st.tokens_in, 0);
    assert_eq!(st.tokens_out, 0);
    assert_eq!(st.cost_usd, 0.0);
    assert_eq!(st.run_started_at_ms, None);
    assert_eq!(st.elapsed_ms, 0);
    // 没有任何 run：耗时段为空（状态行不显示耗时段，不出现 " · " 空档）
    assert_eq!(st.elapsed_label_at(5_000_000), "");
    // context_max 非零，避免 context_pct 计算除零
    assert!(st.context_max > 0);
    assert_eq!(st.context_max, DEFAULT_CONTEXT_MAX);
}

#[test]
fn default_delegates_to_empty() {
    let st = StatusLine::default();
    assert_eq!(st.model, StatusLine::empty().model);
    assert_eq!(st.elapsed_ms, 0);
}

#[test]
fn in_flight_run_reports_elapsed_from_start_to_now() {
    let mut st = StatusLine::empty();
    st.start_run(1_000_000);
    assert_eq!(st.run_started_at_ms, Some(1_000_000));
    // 实时耗时随「当前时刻」增长，不需要定时器驱动
    assert_eq!(st.elapsed_label_at(1_001_500), "1.5s");
    assert_eq!(st.elapsed_label_at(1_060_000), "1m00s");
}

#[test]
fn finishing_a_run_settles_total_elapsed_and_clears_start() {
    let mut st = StatusLine::empty();
    st.start_run(1_000_000);
    st.finish_run(1_072_000);
    assert_eq!(st.elapsed_ms, 72_000);
    assert_eq!(st.run_started_at_ms, None);
    // 结算后耗时段读的是 elapsed_ms，不再随时间变化
    assert_eq!(st.elapsed_label_at(9_999_999), "1m12s");
}

#[test]
fn finishing_with_no_in_flight_run_keeps_the_previous_total() {
    // 关键不变式： daemon 只在 run 真正结束时写 finished_at_ms，
    // 但中断/重试可能让页面收到多次 TurnEnd。重复 finish 不能把已经
    // 结算好的耗时清零——那是用户唯一能看到的一次真实耗时。
    let mut st = StatusLine::empty();
    st.start_run(1_000_000);
    st.finish_run(1_072_000);
    assert_eq!(st.elapsed_ms, 72_000);

    st.finish_run(2_000_000); // 无在飞 run：no-op，不结算也不清零
    assert_eq!(st.elapsed_ms, 72_000);
    assert_eq!(st.run_started_at_ms, None);
    assert_eq!(st.elapsed_label_at(2_000_000), "1m12s");
}

#[test]
fn starting_a_new_run_clears_the_previous_total() {
    let mut st = StatusLine::empty();
    st.start_run(1_000_000);
    st.finish_run(1_072_000);
    assert_eq!(st.elapsed_ms, 72_000);

    // 新 run 开始：旧耗时归零，开始时刻换成新的
    st.start_run(3_000_000);
    assert_eq!(st.elapsed_ms, 0);
    assert_eq!(st.run_started_at_ms, Some(3_000_000));
    assert_eq!(st.elapsed_label_at(3_000_500), "500ms");
}

#[test]
fn finish_before_any_start_is_a_no_op() {
    let mut st = StatusLine::empty();
    st.finish_run(5_000_000);
    assert_eq!(st.elapsed_ms, 0);
    assert_eq!(st.run_started_at_ms, None);
    assert_eq!(st.elapsed_label_at(5_000_000), "");
}

// ── run → task card projection ───────────────────────────────────────────────

#[test]
fn unfinished_run_is_in_progress() {
    assert_eq!(run_task_status(None, None), "in_progress");
    assert_eq!(run_task_status(None, Some("")), "in_progress");
    // daemon 只在写 finished_at_ms 的同一刻写 status，二者理应同步；
    // 防御性地锁定「finished_at_ms 才是终态判据」——已结束但 status
    // 缺失时不能被误标成 done。
    assert_eq!(run_task_status(Some(9_000), None), "blocked");
}

#[test]
fn ok_run_is_done_other_terminals_are_blocked() {
    assert_eq!(run_task_status(Some(9_000), Some("ok")), "done");
    assert_eq!(run_task_status(Some(9_000), Some("failed")), "blocked");
    assert_eq!(
        run_task_status(Some(9_000), Some("spawn_failed")),
        "blocked"
    );
    assert_eq!(run_task_status(Some(9_000), Some("aborted")), "blocked");
    assert_eq!(
        run_task_status(Some(9_000), Some("anything-unknown")),
        "blocked"
    );
}

#[test]
fn in_flight_run_card_marks_itself_running_and_leaves_acceptance_empty() {
    let card = TaskItem::from_run("run-1", 1_000_000, None, None);
    assert_eq!(card.id, "run-1");
    assert_eq!(card.kind, "run");
    assert_eq!(card.status, "in_progress");
    assert_eq!(card.description, "进行中");
    // run 记录没有「验收标准」这一概念：留空，不编造内容
    assert_eq!(card.acceptance, "");
    // 标题是「运行 · <相对时间>」；相对时间随当前时刻变化，只锁前缀
    assert!(card.title.starts_with("运行 · "), "title = {0}", card.title);
}

#[test]
fn finished_run_card_carries_duration_and_terminal_status() {
    let card = TaskItem::from_run("run-2", 1_000_000, Some(1_072_000), Some("ok"));
    assert_eq!(card.id, "run-2");
    assert_eq!(card.status, "done");
    assert!(
        card.description.contains("1m12s"),
        "description = {}",
        card.description
    );
    assert!(
        card.description.contains("ok"),
        "description = {}",
        card.description
    );
    assert_eq!(card.acceptance, "");
}

#[test]
fn failed_run_card_is_blocked_and_shows_status() {
    let card = TaskItem::from_run("run-3", 1_000_000, Some(1_000_500), Some("failed"));
    assert_eq!(card.status, "blocked");
    assert!(card.description.contains("500ms"));
    assert!(card.description.contains("failed"));
}

#[test]
fn negative_or_zero_start_does_not_underflow_the_card() {
    // 防御：时间戳解析/时钟跳变不应让格式化 panic 或回绕。
    // started=-5 被 clamp 到 0，finished=5 → 耗时 5ms（不是 10ms）。
    let card = TaskItem::from_run("run-4", -5, Some(5), Some("ok"));
    assert_eq!(card.status, "done");
    assert_eq!(card.description, "耗时 5ms · ok");
}
