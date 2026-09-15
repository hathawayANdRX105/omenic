//! `stats.summary` 聚合的单元测试（G5/5.7）。
//!
//! 覆盖 `daemon::state::aggregate_stats`（纯函数，注入 `now_ms`，不碰时钟）
//! 与 `RunLedger::stats`（走真实 ledger 文件）。
//!
//! 终态取值以 daemon 实际写入为准：`dispatch.rs` 只写 `"ok"` /
//! `"failed"` / `"spawn_failed"` 三种；`"aborted"` 是协议里保留的第四种
//! （`RunLedger::finish` 接任意串），聚合侧单独计数，故一并覆盖。
//! 未结束的 run 没有 `finished_at_ms`，算 in-flight，不进成功/失败。

use daemon::state::{
    RunLedger, RunRecord, STATS_BUCKETS, STATS_UNAVAILABLE, StatsWindow, aggregate_stats,
};
use std::time::{SystemTime, UNIX_EPOCH};

const HOUR: i64 = 3_600_000;
const DAY: i64 = 24 * HOUR;

/// 固定的聚合参考时刻，让所有断言与真实时钟无关。
const NOW: i64 = 1_800_000_000_000;

/// 构造一条已结束的 run。
fn finished(run_id: &str, session_id: &str, started: i64, dur: i64, status: &str) -> RunRecord {
    RunRecord {
        seq: 0,
        run_id: run_id.to_string(),
        session_id: session_id.to_string(),
        started_at_ms: started,
        finished_at_ms: Some(started + dur),
        status: Some(status.to_string()),
    }
}

/// 构造一条在飞 run（无 `finished_at_ms`，无 status）。
fn in_flight(run_id: &str, session_id: &str, started: i64) -> RunRecord {
    RunRecord {
        seq: 0,
        run_id: run_id.to_string(),
        session_id: session_id.to_string(),
        started_at_ms: started,
        finished_at_ms: None,
        status: None,
    }
}

// ── ① range 过滤边界 ────────────────────────────────────────────────────

/// 24h 窗口是 `started_at_ms >= now - 24h` 的左闭区间：正好落在边界上的
/// run 算「在窗口内」，早 1ms 的被排除。
#[test]
fn range_filter_boundary_is_inclusive_at_window_start() {
    let runs = vec![
        // 恰好在窗口起点 —— 必须计入
        finished("r-on-edge", "s1", NOW - DAY, 1_000, "ok"),
        // 比窗口起点早 1ms —— 必须排除
        finished("r-just-out", "s1", NOW - DAY - 1, 1_000, "ok"),
        // 窗口正中 —— 计入
        finished("r-mid", "s1", NOW - 12 * HOUR, 1_000, "ok"),
    ];

    let s = aggregate_stats(&runs, "24h", NOW);

    assert_eq!(s.total_runs, 2, "只有 on-edge 与 mid 落在 24h 窗口内");
    assert_eq!(s.ok_runs, 2);
    assert_eq!(s.window_start_ms, NOW - DAY);
    assert_eq!(s.range, "24h");
    assert_eq!(s.now_ms, NOW);
}

/// 窗口外的 run 只影响上一窗口计数，不进当前窗口。
#[test]
fn previous_window_counts_only_the_immediately_preceding_span() {
    let runs = vec![
        // 当前 1h 窗口内
        finished("cur-ok", "s1", NOW - 30 * 60_000, 1_000, "ok"),
        // 上一个 1h 窗口（now-2h .. now-1h）
        finished("prev-ok", "s1", NOW - 90 * 60_000, 1_000, "ok"),
        finished("prev-bad", "s1", NOW - 100 * 60_000, 1_000, "failed"),
        // 更早，两个窗口都不算
        finished("ancient", "s1", NOW - 5 * HOUR, 1_000, "ok"),
    ];

    let s = aggregate_stats(&runs, "1h", NOW);

    assert_eq!(s.total_runs, 1);
    assert_eq!(s.prev_total_runs, Some(2), "上一窗口有 2 条");
    assert_eq!(s.prev_ok_runs, Some(1));
    assert_eq!(s.prev_error_runs, Some(1));
}

/// `All` 从最早一条 run 起算，且没有可比的上一窗口。
#[test]
fn all_range_spans_from_earliest_run_and_has_no_previous_window() {
    let earliest = NOW - 90 * DAY;
    let runs = vec![
        finished("old", "s1", earliest, 2_000, "ok"),
        finished("new", "s2", NOW - HOUR, 4_000, "ok"),
    ];

    let s = aggregate_stats(&runs, "All", NOW);

    assert_eq!(s.total_runs, 2, "All 收全部记录");
    assert_eq!(s.window_start_ms, earliest);
    assert_eq!(s.prev_total_runs, None, "All 没有更早的可比窗口");
    assert_eq!(s.prev_ok_runs, None);
    assert_eq!(s.prev_error_runs, None);
    assert_eq!(s.prev_avg_duration_ms, None);
}

/// 未知 range token 回落 24h，不报错（陈旧客户端不能把 daemon 打挂）。
#[test]
fn unknown_range_token_falls_back_to_24h() {
    let runs = vec![finished("r1", "s1", NOW - 2 * HOUR, 1_000, "ok")];

    let s = aggregate_stats(&runs, "bogus-range", NOW);

    assert_eq!(s.window_start_ms, NOW - DAY, "回落到 24h 窗口");
    assert_eq!(s.total_runs, 1);
    assert_eq!(s.range, "bogus-range", "range 字段原样回显请求值");
}

/// 六个 UI token 全部能解析成预期窗口。
#[test]
fn every_ui_range_token_parses_to_expected_window() {
    use daemon::state::parse_stats_range;
    assert_eq!(parse_stats_range("1h"), StatsWindow::Last(HOUR));
    assert_eq!(parse_stats_range("24h"), StatsWindow::Last(DAY));
    assert_eq!(parse_stats_range("7d"), StatsWindow::Last(7 * DAY));
    assert_eq!(parse_stats_range("30d"), StatsWindow::Last(30 * DAY));
    assert_eq!(parse_stats_range("90d"), StatsWindow::Last(90 * DAY));
    // UI 传的是首字母大写的 "All"，解析大小写不敏感
    assert_eq!(parse_stats_range("All"), StatsWindow::All);
    assert_eq!(parse_stats_range("all"), StatsWindow::All);
}

// ── ② 成功 / 失败 / 在飞计数 ────────────────────────────────────────────

/// daemon 实写的三种终态 + 保留的 aborted 各自归类：只有 `"ok"` 算成功，
/// `failed` / `spawn_failed` 归 failed，`aborted` 单独计数，在飞的既不算
/// 成功也不算失败。
#[test]
fn terminal_statuses_are_classified_by_daemon_semantics() {
    let runs = vec![
        finished("a", "s1", NOW - HOUR, 1_000, "ok"),
        finished("b", "s1", NOW - HOUR, 1_000, "ok"),
        finished("c", "s2", NOW - HOUR, 1_000, "failed"),
        finished("d", "s2", NOW - HOUR, 1_000, "spawn_failed"),
        finished("e", "s3", NOW - HOUR, 1_000, "aborted"),
        in_flight("f", "s3", NOW - 60_000),
        in_flight("g", "s4", NOW - 30_000),
    ];

    let s = aggregate_stats(&runs, "24h", NOW);

    assert_eq!(s.total_runs, 7);
    assert_eq!(s.ok_runs, 2, "只有 status == ok 算成功");
    assert_eq!(s.failed_runs, 2, "failed + spawn_failed");
    assert_eq!(s.aborted_runs, 1);
    assert_eq!(s.in_flight_runs, 2, "无 finished_at_ms 的算在飞");
    assert_eq!(s.error_runs(), 3, "error_runs = failed + aborted");
    assert_eq!(
        s.ok_runs + s.failed_runs + s.aborted_runs + s.in_flight_runs,
        s.total_runs,
        "四类计数必须正好覆盖 total，无重复无遗漏"
    );
}

/// 已结束但 status 缺失（旧记录 / torn 写入）不能被当成成功。
#[test]
fn finished_run_without_status_counts_as_failed_not_ok() {
    let runs = vec![RunRecord {
        seq: 1,
        run_id: "r-no-status".to_string(),
        session_id: "s1".to_string(),
        started_at_ms: NOW - HOUR,
        finished_at_ms: Some(NOW - HOUR + 500),
        status: None,
    }];

    let s = aggregate_stats(&runs, "24h", NOW);

    assert_eq!(s.ok_runs, 0, "status 缺失不得计为成功");
    assert_eq!(s.failed_runs, 1);
    assert_eq!(s.in_flight_runs, 0, "有 finished_at_ms 就不是在飞");
}

/// 活跃会话按 `session_id` 去重，空 session_id 不计入。
#[test]
fn active_sessions_are_deduplicated_and_ignore_empty_ids() {
    let runs = vec![
        finished("r1", "s1", NOW - HOUR, 1_000, "ok"),
        finished("r2", "s1", NOW - HOUR, 1_000, "ok"),
        finished("r3", "s2", NOW - HOUR, 1_000, "ok"),
        // 无归属 run（worker.prompt 不带 session_id 时的形态）
        finished("r4", "", NOW - HOUR, 1_000, "ok"),
    ];

    let s = aggregate_stats(&runs, "24h", NOW);

    assert_eq!(s.active_sessions, 2, "s1 去重后只算一次，空 id 不计");
}

// ── ③ 派生指标 ──────────────────────────────────────────────────────────

/// 平均耗时只对已结束的 run 求均值，在飞 run 不拉低分母。
#[test]
fn average_duration_covers_only_finished_runs() {
    let runs = vec![
        finished("a", "s1", NOW - HOUR, 1_000, "ok"),
        finished("b", "s1", NOW - HOUR, 3_000, "failed"),
        // 在飞：无耗时可算，必须不进平均
        in_flight("c", "s1", NOW - 60_000),
    ];

    let s = aggregate_stats(&runs, "24h", NOW);

    assert_eq!(
        s.avg_duration_ms,
        Some(2_000.0),
        "(1000 + 3000) / 2，在飞 run 不参与"
    );
}

/// 时钟回拨导致的负耗时记录被跳过，不污染平均值。
#[test]
fn negative_duration_records_are_skipped_in_average() {
    let runs = vec![
        finished("good", "s1", NOW - HOUR, 2_000, "ok"),
        // finished 早于 started（时钟回拨）
        RunRecord {
            seq: 2,
            run_id: "bad-clock".to_string(),
            session_id: "s1".to_string(),
            started_at_ms: NOW - HOUR,
            finished_at_ms: Some(NOW - HOUR - 5_000),
            status: Some("ok".to_string()),
        },
    ];

    let s = aggregate_stats(&runs, "24h", NOW);

    assert_eq!(s.avg_duration_ms, Some(2_000.0), "负耗时不进均值");
    assert_eq!(s.ok_runs, 2, "但状态计数照旧");
    // feed 里该行的耗时应为 None（无法给出有意义的耗时）
    let bad = s
        .recent
        .iter()
        .find(|r| r.run_id == "bad-clock")
        .expect("bad-clock 应出现在 recent 里");
    assert_eq!(bad.duration_ms, None);
}

/// 窗口内一条也没结束时平均耗时是 None，不是 0（0 会被 UI 当成真实的
/// 「瞬间完成」渲染）。
#[test]
fn average_duration_is_none_when_nothing_finished() {
    let runs = vec![in_flight("a", "s1", NOW - 60_000)];

    let s = aggregate_stats(&runs, "24h", NOW);

    assert_eq!(s.avg_duration_ms, None);
    assert_eq!(s.total_runs, 1);
    assert_eq!(s.in_flight_runs, 1);
}

/// 每小时运行数 = 窗口内 run 数 / 窗口小时数。
#[test]
fn runs_per_hour_divides_by_window_length() {
    // 24h 窗口里 48 条 → 每小时 2 条
    let runs: Vec<RunRecord> = (0..48)
        .map(|i| finished(&format!("r{i}"), "s1", NOW - DAY + i * 60_000, 100, "ok"))
        .collect();

    let s = aggregate_stats(&runs, "24h", NOW);

    assert_eq!(s.total_runs, 48);
    let rph = s.runs_per_hour.expect("24h 窗口长度非零，应给出速率");
    assert!((rph - 2.0).abs() < 1e-9, "48 / 24 = 2，实际 {rph}");
}

/// 吞吐序列固定 12 桶、覆盖整个窗口且桶计数之和等于窗口内 run 数。
#[test]
fn throughput_buckets_tile_the_window_and_sum_to_total() {
    let runs = vec![
        // 第一桶（窗口起点）
        finished("first", "s1", NOW - DAY, 100, "ok"),
        // 窗口正中
        finished("mid", "s1", NOW - 12 * HOUR, 100, "ok"),
        // 恰好在 now：必须落进最后一桶，不能被丢掉
        finished("at-now", "s1", NOW, 0, "ok"),
    ];

    let s = aggregate_stats(&runs, "24h", NOW);

    assert_eq!(s.throughput.len(), STATS_BUCKETS, "桶数固定");
    assert_eq!(s.throughput[0].start_ms, NOW - DAY, "第一桶从窗口起点开始");
    assert_eq!(
        s.throughput[STATS_BUCKETS - 1].end_ms,
        NOW,
        "最后一桶结束于 now"
    );
    // 桶必须首尾相接，无缝隙无重叠
    for pair in s.throughput.windows(2) {
        assert_eq!(pair[0].end_ms, pair[1].start_ms, "桶之间不得有缝隙");
    }
    let bucketed: u64 = s.throughput.iter().map(|b| b.runs).sum();
    assert_eq!(
        bucketed, s.total_runs,
        "桶计数之和须等于窗口内 run 数（含落在 now 上的那条）"
    );
    assert!(
        s.throughput.iter().all(|b| !b.label.is_empty()),
        "每桶都要有轴标签"
    );
}

/// recent feed 按开始时间倒序，在飞 run 报 `running` 且无耗时。
#[test]
fn recent_feed_is_newest_first_and_marks_in_flight_runs() {
    let runs = vec![
        finished("oldest", "s1", NOW - 3 * HOUR, 1_000, "ok"),
        finished("middle", "s1", NOW - 2 * HOUR, 2_000, "failed"),
        in_flight("newest", "s1", NOW - HOUR),
    ];

    let s = aggregate_stats(&runs, "24h", NOW);

    let ids: Vec<&str> = s.recent.iter().map(|r| r.run_id.as_str()).collect();
    assert_eq!(ids, vec!["newest", "middle", "oldest"], "最新的排最前");

    assert_eq!(s.recent[0].status, "running", "在飞 run 报 running");
    assert_eq!(s.recent[0].duration_ms, None, "在飞 run 没有耗时");
    assert_eq!(s.recent[1].status, "failed");
    assert_eq!(s.recent[1].duration_ms, Some(2_000));
    assert_eq!(s.recent[2].duration_ms, Some(1_000));
}

/// 无数据源的指标必须出现在 `unavailable` 里——这是「不编造数值」的契约，
/// UI 据此隐藏 / 标注对应卡片。
#[test]
fn metrics_without_a_data_source_are_reported_unavailable() {
    let s = aggregate_stats(&[], "24h", NOW);

    // 遍历常量本身而不是手抄一份：漏掉一个键（比如 agent_token_split）
    // 会让「不编造数值」的契约出现静默缺口——删掉常量里的任一项此测试
    // 仍会过，但 UI 就会少标注一张无数据卡。
    for key in STATS_UNAVAILABLE {
        assert!(
            s.is_unavailable(key),
            "{key} 在 run ledger 里没有数据源，必须列进 unavailable"
        );
    }
    assert!(
        !s.is_unavailable("total_runs"),
        "run 数是真实可算的，不该被标记不可用"
    );
}

// ── ④ 空 ledger ────────────────────────────────────────────────────────

/// 空输入不 panic，计数全零，可选指标为 None（而不是伪造的 0）。
#[test]
fn empty_input_yields_zeroes_without_panicking() {
    let s = aggregate_stats(&[], "24h", NOW);

    assert_eq!(s.total_runs, 0);
    assert_eq!(s.ok_runs, 0);
    assert_eq!(s.failed_runs, 0);
    assert_eq!(s.aborted_runs, 0);
    assert_eq!(s.in_flight_runs, 0);
    assert_eq!(s.active_sessions, 0);
    assert_eq!(s.avg_duration_ms, None, "没有样本时不能报 0ms");
    assert!(s.recent.is_empty());
    assert_eq!(s.prev_total_runs, Some(0), "定长窗口仍有可比的上一窗口");
    // 定长窗口长度非零，速率是有意义的 0
    assert_eq!(s.runs_per_hour, Some(0.0));
    assert_eq!(s.throughput.len(), STATS_BUCKETS);
    assert!(s.throughput.iter().all(|b| b.runs == 0));
}

/// 空 ledger 上的 `All`：窗口长度为 0，不能除零，速率与折线都退化为空。
#[test]
fn empty_all_range_has_zero_length_window_and_no_series() {
    let s = aggregate_stats(&[], "All", NOW);

    assert_eq!(s.total_runs, 0);
    assert_eq!(s.window_start_ms, NOW, "无记录时窗口起点退化到 now");
    assert_eq!(s.runs_per_hour, None, "零长窗口不得给出速率（避免除零）");
    assert!(s.throughput.is_empty(), "零长窗口没有可画的桶");
}

// ── RunLedger::stats 端到端（真实 ledger 文件） ─────────────────────────

/// 走真实 `RunLedger`（写文件 + start/finish），确认 `stats()` 读到的是
/// 落盘后的记录，且与 `aggregate_stats` 结论一致。
#[test]
fn ledger_stats_aggregates_persisted_records() {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket = dir.path().join("daemon.sock");
    let ledger = RunLedger::open_for_socket(&socket).expect("open ledger");

    let now = now_ms_wall();

    // 空 ledger 先测一遍：不 panic、全零
    let empty = ledger.stats("24h", now);
    assert_eq!(empty.total_runs, 0);
    assert_eq!(empty.avg_duration_ms, None);

    ledger.start("r-ok", "sess-a", now - 10_000).expect("start");
    ledger.finish("r-ok", now - 8_000, "ok").expect("finish");

    ledger
        .start("r-failed", "sess-a", now - 6_000)
        .expect("start");
    ledger
        .finish("r-failed", now - 2_000, "failed")
        .expect("finish");

    // 只 start 不 finish：在飞
    ledger
        .start("r-live", "sess-b", now - 1_000)
        .expect("start");

    let s = ledger.stats("24h", now);

    assert_eq!(s.total_runs, 3);
    assert_eq!(s.ok_runs, 1);
    assert_eq!(s.failed_runs, 1);
    assert_eq!(s.in_flight_runs, 1);
    assert_eq!(s.active_sessions, 2, "sess-a / sess-b");
    assert_eq!(
        s.avg_duration_ms,
        Some(3_000.0),
        "(2000 + 4000) / 2；在飞的 r-live 不参与"
    );

    // 与纯函数走同一份数据应得同一结论
    let direct = aggregate_stats(&ledger.list(), "24h", now);
    assert_eq!(direct, s);
}

/// 1h 窗口把更早的落盘记录排除掉（ledger 里有，但不该进这个窗口）。
#[test]
fn ledger_stats_respects_the_requested_window() {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket = dir.path().join("daemon.sock");
    let ledger = RunLedger::open_for_socket(&socket).expect("open ledger");

    let now = now_ms_wall();

    ledger.start("recent", "s1", now - 10 * 60_000).expect("s");
    ledger.finish("recent", now - 9 * 60_000, "ok").expect("f");
    // 3 小时前，落在 1h 窗口外
    ledger.start("stale", "s1", now - 3 * HOUR).expect("s");
    ledger
        .finish("stale", now - 3 * HOUR + 500, "ok")
        .expect("f");

    assert_eq!(ledger.list().len(), 2, "两条都在 ledger 里");

    let hour = ledger.stats("1h", now);
    assert_eq!(hour.total_runs, 1, "1h 窗口只收 recent");

    let all = ledger.stats("All", now);
    assert_eq!(all.total_runs, 2, "All 收两条");
}

/// 当前墙上时钟（毫秒）。`RunLedger` 的 API 收绝对时间戳，这里用真实
/// 时刻构造记录，再把同一个 `now` 传给 `stats()`，聚合仍是确定的。
fn now_ms_wall() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
