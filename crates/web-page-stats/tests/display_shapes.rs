//! Display-shape logic: the pure builders that turn a [`StatsSummary`] into
//! KPI cards, status bars, feed rows, and delta chips.

use web_client::daemon::{StatsBucket, StatsRecentRun, StatsSummary};
use web_page_stats::{
    Tone, build_band, build_feed, build_kpis, build_points, build_status_bars,
    build_unavailable_note, delta_avg, delta_count, error_rate, format_duration, tone_for,
};

fn summary() -> StatsSummary {
    StatsSummary {
        range: "24h".into(),
        now_ms: 0,
        window_start_ms: 0,
        total_runs: 0,
        ok_runs: 0,
        failed_runs: 0,
        aborted_runs: 0,
        in_flight_runs: 0,
        avg_duration_ms: None,
        active_sessions: 0,
        runs_per_hour: None,
        throughput: vec![],
        recent: vec![],
        prev_total_runs: None,
        prev_ok_runs: None,
        prev_error_runs: None,
        prev_avg_duration_ms: None,
        unavailable: vec![],
    }
}

#[test]
fn kpis_without_summary_keep_five_card_skeleton() {
    let kpis = build_kpis(None);
    assert_eq!(
        kpis.iter().map(|k| k.label.as_str()).collect::<Vec<_>>(),
        ["运行数", "成功数", "错误率", "平均耗时", "活跃会话"]
    );
    assert!(kpis.iter().all(|k| k.value == "—"));
    let deltas: Vec<_> = kpis.iter().map(|k| k.delta.as_str()).collect();
    assert_eq!(deltas, ["无数据"; 5]);
    assert!(kpis.iter().all(|k| k.tone == Tone::Flat));
}

#[test]
fn kpis_carry_deltas_from_previous_window() {
    let mut s = summary();
    s.total_runs = 10;
    s.ok_runs = 7;
    s.failed_runs = 2;
    s.aborted_runs = 1;
    s.active_sessions = 3;
    s.avg_duration_ms = Some(1000.0);
    s.prev_total_runs = Some(20);
    s.prev_ok_runs = Some(10);
    s.prev_error_runs = Some(5);
    s.prev_avg_duration_ms = Some(2000.0);

    let kpis = build_kpis(Some(&s));
    assert_eq!(kpis[0].value, "10");
    assert_eq!(
        (kpis[0].delta.as_str(), kpis[0].tone),
        ("-50.0%", Tone::Bad)
    );
    assert_eq!(kpis[1].value, "7");
    assert_eq!(
        (kpis[1].delta.as_str(), kpis[1].tone),
        ("-30.0%", Tone::Bad)
    );
    // Error rate over terminal runs only, drop is good news.
    assert_eq!(kpis[2].value, "30.0%");
    assert_eq!(
        (kpis[2].delta.as_str(), kpis[2].tone),
        ("-40.0%", Tone::Good)
    );
    assert_eq!(kpis[3].value, "1.0s");
    assert_eq!(
        (kpis[3].delta.as_str(), kpis[3].tone),
        ("-50.0%", Tone::Good)
    );
    assert_eq!(
        (kpis[4].value.as_str(), kpis[4].delta.as_str()),
        ("3", "本窗口")
    );
}

#[test]
fn delta_count_has_three_states() {
    // No comparable window (All range) and zero previous baseline are both
    // flat: the chip must not fabricate a percentage.
    let chip = delta_count(5, None, true);
    assert_eq!((chip.delta.as_str(), chip.tone), ("无可比窗口", Tone::Flat));
    let chip = delta_count(5, Some(0), true);
    assert_eq!((chip.delta.as_str(), chip.tone), ("环比无基数", Tone::Flat));
    let chip = delta_count(15, Some(10), true);
    assert_eq!((chip.delta.as_str(), chip.tone), ("+50.0%", Tone::Good));
}

#[test]
fn delta_avg_falls_back_to_flat_without_baseline() {
    assert_eq!(
        (
            delta_avg(None, Some(5.0)).delta.as_str(),
            delta_avg(None, Some(5.0)).tone
        ),
        ("无可比窗口", Tone::Flat)
    );
    assert_eq!(
        (
            delta_avg(Some(5.0), Some(0.0)).delta.as_str(),
            delta_avg(Some(5.0), Some(0.0)).tone
        ),
        ("无可比窗口", Tone::Flat)
    );
}

#[test]
fn error_rate_excludes_in_flight_runs() {
    let mut s = summary();
    s.ok_runs = 7;
    s.failed_runs = 2;
    s.aborted_runs = 1;
    s.in_flight_runs = 5; // must not dilute the denominator
    assert_eq!(error_rate(&s), Some(30.0));

    let empty = summary();
    assert_eq!(error_rate(&empty), None);
}

#[test]
fn status_bars_compute_percentages_of_terminal_total() {
    let mut s = summary();
    s.ok_runs = 3;
    s.failed_runs = 1;
    s.in_flight_runs = 1;

    let bars = build_status_bars(Some(&s));
    assert_eq!(bars.len(), 4);
    assert_eq!(
        (bars[0].label.as_str(), bars[0].count.as_str(), bars[0].pct),
        ("成功", "3", 60.0)
    );
    assert_eq!(
        (bars[1].label.as_str(), bars[1].count.as_str(), bars[1].pct),
        ("失败", "1", 20.0)
    );
    assert_eq!((bars[2].label.as_str(), bars[2].pct), ("已中止", 0.0));
    assert_eq!((bars[3].label.as_str(), bars[3].pct), ("进行中", 20.0));

    let none = build_status_bars(None);
    assert!(none.iter().all(|b| b.count == "0" && b.pct == 0.0));
}

#[test]
fn points_map_throughput_buckets() {
    let mut s = summary();
    s.throughput = vec![
        StatsBucket {
            start_ms: 0,
            end_ms: 1,
            label: "00:00".into(),
            runs: 3,
        },
        StatsBucket {
            start_ms: 1,
            end_ms: 2,
            label: "00:01".into(),
            runs: 7,
        },
    ];

    assert_eq!(
        build_points(Some(&s))
            .iter()
            .map(|p| p.value)
            .collect::<Vec<_>>(),
        [3.0, 7.0]
    );
    assert!(build_points(None).is_empty());
}

#[test]
fn feed_maps_status_to_color_classes() {
    let mut s = summary();
    s.recent = vec![
        StatsRecentRun {
            run_id: "r1".into(),
            session_id: "s1".into(),
            started_at_ms: 0,
            duration_ms: Some(820),
            status: "ok".into(),
        },
        StatsRecentRun {
            run_id: "r2".into(),
            session_id: "s1".into(),
            started_at_ms: 1,
            duration_ms: None,
            status: "running".into(),
        },
        StatsRecentRun {
            run_id: "r3".into(),
            session_id: "s2".into(),
            started_at_ms: 2,
            duration_ms: Some(5000),
            status: "spawn_failed".into(),
        },
    ];

    let feed = build_feed(Some(&s));
    assert_eq!(feed[0].status_class, "text-success-2");
    assert_eq!(feed[0].duration, "820ms");
    assert_eq!(feed[1].status_class, "text-brand");
    assert_eq!(feed[1].duration, "进行中");
    // Unrecognized terminal states render as errors, never as success.
    assert_eq!(feed[2].status_class, "text-danger");
    assert_eq!(feed[2].duration, "5.0s");
}

#[test]
fn unavailable_note_translates_metric_keys() {
    let mut s = summary();
    s.unavailable = vec!["tokens".into(), "cost".into(), "agent_token_split".into()];
    assert_eq!(
        build_unavailable_note(Some(&s)),
        "暂无数据源（需 token 计量）：token 用量、费用、主副 agent token 分布。"
    );
    assert_eq!(build_unavailable_note(Some(&summary())), "");
    assert_eq!(
        build_unavailable_note(None),
        "未连接 daemon：统计数据不可用。"
    );
}

#[test]
fn band_values_fall_back_to_dash() {
    let band = build_band(None);
    assert_eq!(band.len(), 7);
    assert!(band.iter().all(|b| b.value == "—"));

    let mut s = summary();
    s.runs_per_hour = Some(1.5);
    let band = build_band(Some(&s));
    assert_eq!(band[0].value, "1.50");
    assert_eq!(band[1].value, "—"); // no avg duration
}

#[test]
fn tone_for_zero_change_is_always_flat() {
    assert_eq!(tone_for(0.0, true), Tone::Flat);
    assert_eq!(tone_for(0.0, false), Tone::Flat);
    assert_eq!(tone_for(5.0, false), Tone::Bad);
    assert_eq!(tone_for(-5.0, false), Tone::Good);
}

#[test]
fn format_duration_tiers() {
    assert_eq!(format_duration(820.0), "820ms");
    assert_eq!(format_duration(12_400.0), "12.4s");
    assert_eq!(format_duration(192_000.0), "3m12s");
    assert_eq!(format_duration(f64::NAN), "—");
    assert_eq!(format_duration(-1.0), "—");
}
