//! 数据统计页（dsh 设计语言重刷）：KPI 卡、指标带、三列主体。
//!
//! G5/5.7：数据源从 `omenic-web-mock` 换成 daemon 的 `stats.summary`
//! （run ledger 聚合，见 `daemon::state::aggregate_stats`）。
//!
//! 两条硬约束写在这里，避免后续误改：
//!
//! 1. **不编造数值**。run ledger 只记 `run_id / session_id /
//!    started_at_ms / finished_at_ms / status`，没有 token、价格、缓存、
//!    首字延迟，也没有 model / provider / 主副 agent 归属。这些指标在
//!    [`StatsSummary::unavailable`] 里由 daemon 显式列出，页面**只渲染
//!    真实可算的量**，不可算的写进「暂无数据源」说明行（ROADMAP 5.7
//!    的「无真数据则隐藏该卡」，完整数据源要等 C8 token 计量）。
//! 2. **渲染路径不能同步 RPC**。Dioxus LiveView 跑在 tokio 里，直接在
//!    渲染/effect 里调阻塞客户端会 panic（runtime within a runtime），
//!    所以所有请求都套 `std::thread::spawn(...).join()`，与
//!    `page-workspace` 的 `list_sessions` / `load_messages` 同一模式。
//!
//! 无 daemon / RPC 失败时 `summary` 恒为 `None`：KPI 归零、图表空、feed
//! 空，容器与样式全部保留（`specs/ui/stats.yaml` 的锚点是 class 片段，
//! 空态不许把带锚点的节点整块删掉）。

use dioxus::prelude::*;
use omenic_web_client::daemon::{StatsSummary, WebDaemon};

/// 时间范围切换项，与 `daemon::state::parse_stats_range` 认的 token 对齐。
const RANGES: [&str; 6] = ["1h", "24h", "7d", "30d", "90d", "All"];

/// 图表折线 / 数据点的品牌蓝。
const BRAND: &str = "#679efe";

// ── 页面内部展示形状 ────────────────────────────────────────────────────
// 刻意不复用 `omenic_web_state::types::{KpiCard, SubMetric, ...}`：那些
// 形状是给 mock 的字符串数据设计的（delta 只有正负两态），这里需要第三
// 态「无可比窗口」；且 types.rs 由并行任务在改，不碰为好。

/// KPI / delta chip 的三态色调。`Flat` = 没有可比的上一窗口（All 范围或
/// 上一窗口零 run），此时不能声称涨跌。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    Good,
    Bad,
    Flat,
}

impl Tone {
    fn chip_class(self) -> &'static str {
        match self {
            Tone::Good => "bg-chip-success text-success-2",
            Tone::Bad => "bg-chip-danger text-danger",
            Tone::Flat => "bg-layer-2 text-label-3",
        }
    }
}

/// 一张 KPI 卡。
#[derive(Debug, Clone, PartialEq)]
struct Kpi {
    label: String,
    value: String,
    delta: String,
    tone: Tone,
}

/// 环比 chip：KPI 卡右下角的涨跌标识。独立成类型，而不是复用 `Kpi`——
/// 一个 label/value 全空的 Kpi 是不完整的数据，只能靠 `..` 结构更新语法
/// 临时拼装，单独传出去会产生「空卡片」这种无效状态。
#[derive(Debug, Clone, PartialEq)]
struct DeltaChip {
    delta: String,
    tone: Tone,
}

impl Kpi {
    fn new(label: impl Into<String>, value: impl Into<String>, chip: DeltaChip) -> Kpi {
        Kpi {
            label: label.into(),
            value: value.into(),
            delta: chip.delta,
            tone: chip.tone,
        }
    }
}

/// 指标带的一格。
#[derive(Debug, Clone, PartialEq)]
struct Band {
    label: String,
    value: String,
}

/// 运行状态分布的一条（复用原 Agent 分布条的视觉）。
#[derive(Debug, Clone, PartialEq)]
struct StatusBar {
    label: String,
    count: String,
    pct: f64,
    color: String,
}

/// 吞吐折线的一个点。
#[derive(Debug, Clone, PartialEq)]
struct Point {
    value: f64,
}

/// 最近运行 feed 的一行。
#[derive(Debug, Clone, PartialEq)]
struct Feed {
    run_id: String,
    session_id: String,
    duration: String,
    status: String,
    status_class: String,
}

#[component]
pub fn StatsView() -> Element {
    let mut selected_range = use_signal(|| "24h".to_string());

    // daemon 探测只做一次：socket 由 `OMENIC_DAEMON_SOCKET` / 平台配置目录
    // 决定（见 `WebDaemon::from_env_or_default`），不必读配置文件。
    // ping 不通 → None → 全页空态。
    let daemon = use_signal(|| {
        // 探测线程的 panic 不静默吞：`.ok()` 会把 JoinError 抹成 None，
        // 于是 socket 解析或 ping 里的真 panic 只表现为「无 daemon」的空
        // 页，无从排查。失败仍是 None（空态），但先留下痕迹。
        match std::thread::spawn(|| WebDaemon::from_env_or_default().filter(|d| d.ping())).join() {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[web] stats daemon probe thread panicked: {e:?}");
                None
            }
        }
    });

    // `stats.summary` 结果。None = 无 daemon 或 RPC 失败（空态，不回退假数据）。
    let mut summary: Signal<Option<StatsSummary>> = use_signal(|| None);

    // 范围变化即重取。阻塞 RPC 放线程里 join，不在渲染线程上跑。
    use_effect(move || {
        let range = selected_range();
        let Some(d) = daemon() else {
            summary.set(None);
            return;
        };
        let fetched = std::thread::spawn(move || d.stats_summary(&range).ok())
            .join()
            .ok()
            .flatten();
        summary.set(fetched);
    });

    let snapshot = summary.read().clone();
    let kpis = build_kpis(snapshot.as_ref());
    let band = build_band(snapshot.as_ref());
    let bars = build_status_bars(snapshot.as_ref());
    let points = build_points(snapshot.as_ref());
    let feed = build_feed(snapshot.as_ref());
    let unavailable_note = build_unavailable_note(snapshot.as_ref());
    let subtitle = match snapshot.as_ref() {
        Some(s) => format!(
            "当前视图范围：最近 {} · 共 {} 次运行（来自 run ledger）",
            selected_range(),
            s.total_runs
        ),
        None => format!(
            "当前视图范围：最近 {} · 未连接 daemon，暂无统计数据",
            selected_range()
        ),
    };

    rsx! {
        div { class: "flex-1 overflow-y-auto flex flex-col gap-6 px-10 pt-8 pb-14 max-w-[1200px] w-full mx-auto",
            // 头部
            div { class: "flex items-center justify-between pb-5 border-b border-b1",
                div {
                    h1 { class: "text-[22px] leading-7 font-semibold text-label tracking-tight m-0", "数据统计" }
                    p { class: "text-[13px] leading-5 text-label-3 mt-1 m-0", "{subtitle}" }
                }
                div { class: "flex gap-0.5 bg-layer-1 border border-b1 rounded-[10px] p-0.5",
                    for r in RANGES {
                        {
                            let r_str = r.to_string();
                            let class = if selected_range() == r {
                                "px-3.5 h-7 flex items-center text-[13px] rounded-lg font-medium bg-selector text-label cursor-pointer transition-colors border-none"
                            } else {
                                "px-3.5 h-7 flex items-center text-[13px] rounded-lg text-label-3 hover:text-label transition-colors cursor-pointer border-none bg-transparent"
                            };
                            rsx! {
                                button {
                                    key: "{r}",
                                    class: "{class}",
                                    onclick: move |_| selected_range.set(r_str.clone()),
                                    "{r}"
                                }
                            }
                        }
                    }
                }
            }

            // KPI 卡（五列固定：空态渲染零值/占位，不删节点）
            div { class: "grid grid-cols-5 gap-3.5",
                for kpi in kpis.iter() {
                    KpiCardView { key: "{kpi.label}", kpi: kpi.clone() }
                }
            }

            // 指标带
            div { class: "grid grid-cols-7 gap-3 bg-layer-1 border border-b1 rounded-xl px-5 py-3.5",
                for sm in band.iter() {
                    div { key: "{sm.label}", class: "flex flex-col gap-1",
                        div { class: "text-[10px] leading-4 font-semibold text-caption font-mono uppercase tracking-wider", "{sm.label}" }
                        div { class: "text-[15px] leading-5 font-semibold text-label font-mono", "{sm.value}" }
                    }
                }
            }

            // 主体三列
            div { class: "grid grid-cols-[320px_1fr_340px] gap-4 items-stretch",
                div { class: "bg-layer-1 border border-b1 rounded-xl p-5 flex flex-col gap-4",
                    h3 { class: "text-[14px] leading-5 font-medium text-label m-0", "运行状态分布" }
                    for bar in bars.iter() {
                        StatusBarRow { key: "{bar.label}", bar: bar.clone() }
                    }
                    p { class: "text-[11px] leading-4 text-caption m-0", "{unavailable_note}" }
                }

                div { class: "bg-layer-1 border border-b1 rounded-xl p-5 flex flex-col gap-4",
                    h3 { class: "text-[14px] leading-5 font-medium text-label m-0", "吞吐趋势（运行数 · {selected_range()}）" }
                    ThroughputChart { points: points.clone() }
                }

                div { class: "bg-layer-1 border border-b1 rounded-xl p-5 flex flex-col gap-4",
                    h3 { class: "text-[14px] leading-5 font-medium text-label m-0", "最近运行 Feed" }
                    div { class: "flex flex-col gap-2 overflow-y-auto max-h-[320px]",
                        if feed.is_empty() {
                            div { class: "text-[12px] leading-5 text-label-3", "窗口内没有运行记录" }
                        }
                        for item in feed.iter() {
                            FeedRow { key: "{item.run_id}", item: item.clone() }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn KpiCardView(kpi: Kpi) -> Element {
    let delta_class = kpi.tone.chip_class();
    rsx! {
        div { class: "bg-layer-1 border border-b1 rounded-xl px-4 py-4 flex flex-col gap-2 hover:border-b2 transition-colors",
            div { class: "text-[12px] leading-5 text-label-3 font-medium", "{kpi.label}" }
            div { class: "text-[26px] leading-7 font-semibold text-label font-mono", "{kpi.value}" }
            span { class: "{delta_class} text-[11px] leading-4 font-semibold px-2 py-0.5 rounded-md w-fit font-mono", "{kpi.delta}" }
        }
    }
}

#[component]
fn StatusBarRow(bar: StatusBar) -> Element {
    rsx! {
        div { class: "flex flex-col gap-2 px-2.5 py-2 bg-base border border-b1 rounded-lg",
            div { class: "flex items-center justify-between text-[12px] leading-5",
                span { class: "font-medium text-label", "{bar.label}" }
                div { class: "flex items-center gap-2",
                    span { class: "text-label-3 font-mono text-[11px]", "{bar.count}" }
                    span { class: "font-semibold text-brand font-mono text-[11px]", "{bar.pct:.1}%" }
                }
            }
            div { class: "h-1.5 bg-layer-2 rounded-full overflow-hidden",
                div { class: "h-full rounded-full", style: "width: {bar.pct}%; background: {bar.color}" }
            }
        }
    }
}

#[component]
fn ThroughputChart(points: Vec<Point>) -> Element {
    // 空序列（无 daemon / 窗口无数据）走同一条路径：polyline 点串为空，
    // SVG 画布仍在，不 panic。
    let max_val = points
        .iter()
        .map(|p| p.value)
        .fold(0.0_f64, f64::max)
        .max(1.0);
    let width = 400.0_f64;
    let height = 180.0_f64;
    let pad = 12.0_f64;
    let step = if points.len() > 1 {
        (width - pad * 2.0) / (points.len() - 1) as f64
    } else {
        0.0
    };

    let path_points: Vec<String> = points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let x = pad + i as f64 * step;
            let y = height - pad - (p.value / max_val) * (height - pad * 2.0);
            format!("{x:.1},{y:.1}")
        })
        .collect();
    let polyline = path_points.join(" ");

    rsx! {
        svg {
            class: "w-full h-[180px] overflow-visible",
            view_box: "0 0 {width} {height}",
            fill: "none",
            polyline {
                points: "{polyline}",
                stroke: "#679efe",
                stroke_width: "1.5",
                stroke_linecap: "round",
                stroke_linejoin: "round",
            }
            for (i, p) in points.iter().enumerate() {
                {
                    let x = pad + i as f64 * step;
                    let y = height - pad - (p.value / max_val) * (height - pad * 2.0);
                    rsx! {
                        circle {
                            key: "pt-{i}",
                            cx: "{x:.1}",
                            cy: "{y:.1}",
                            r: "2.5",
                            fill: "{BRAND}",
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn FeedRow(item: Feed) -> Element {
    rsx! {
        div { class: "flex items-center justify-between px-3 py-2 bg-base border border-b1 rounded-lg text-[12px] hover:bg-ihover transition-colors",
            div { class: "flex items-center gap-2 min-w-0",
                span { class: "font-medium text-brand-300 font-mono text-[11px] truncate", "{item.run_id}" }
                span { class: "text-[10px] text-label-3 bg-layer-2 px-1.5 py-px rounded-md truncate", "{item.session_id}" }
            }
            div { class: "flex items-center gap-2.5 font-mono text-[11px] shrink-0",
                span { class: "text-label-3", "{item.duration}" }
                span { class: "{item.status_class} font-medium", "{item.status}" }
            }
        }
    }
}

// ── 聚合结果 → 展示形状 ─────────────────────────────────────────────────

/// 五张 KPI 卡，全部来自 run ledger 可算的量（原 mock 的费用 / 缓存节省 /
/// 缓存率三张卡没有数据源，直接不再出现——见 [`build_unavailable_note`]）。
/// `None`（无 daemon）时保持五张卡的骨架，数值归零、delta 占位 `—`。
fn build_kpis(s: Option<&StatsSummary>) -> Vec<Kpi> {
    let Some(s) = s else {
        return ["运行数", "成功数", "错误率", "平均耗时", "活跃会话"]
            .iter()
            .map(|label| Kpi {
                label: (*label).to_string(),
                value: "—".to_string(),
                delta: "无数据".to_string(),
                tone: Tone::Flat,
            })
            .collect();
    };

    let errors = s.error_runs();
    vec![
        Kpi::new(
            "运行数",
            s.total_runs.to_string(),
            delta_count(s.total_runs, s.prev_total_runs, true),
        ),
        Kpi::new(
            "成功数",
            s.ok_runs.to_string(),
            delta_count(s.ok_runs, s.prev_ok_runs, true),
        ),
        Kpi::new(
            "错误率",
            match error_rate(s) {
                Some(p) => format!("{p:.1}%"),
                None => "—".to_string(),
            },
            // 错误变多是坏事，故 `higher_is_good = false`。
            delta_count(errors, s.prev_error_runs, false),
        ),
        Kpi::new(
            "平均耗时",
            match s.avg_duration_ms {
                Some(ms) => format_duration(ms),
                None => "—".to_string(),
            },
            delta_avg(s.avg_duration_ms, s.prev_avg_duration_ms),
        ),
        Kpi {
            label: "活跃会话".to_string(),
            value: s.active_sessions.to_string(),
            delta: "本窗口".to_string(),
            tone: Tone::Flat,
        },
    ]
}

/// 七格指标带，同样全为真实可算量。原 mock 的四格 token / TTFT 指标换成
/// 运行终态拆分与吞吐速率。
fn build_band(s: Option<&StatsSummary>) -> Vec<Band> {
    const LABELS: [&str; 7] = [
        "RUNS/H",
        "AVG LATENCY",
        "OK",
        "FAILED",
        "ABORTED",
        "IN FLIGHT",
        "SESSIONS",
    ];
    let Some(s) = s else {
        return LABELS
            .iter()
            .map(|label| Band {
                label: (*label).to_string(),
                value: "—".to_string(),
            })
            .collect();
    };
    let values = [
        match s.runs_per_hour {
            Some(v) => format!("{v:.2}"),
            None => "—".to_string(),
        },
        match s.avg_duration_ms {
            Some(ms) => format_duration(ms),
            None => "—".to_string(),
        },
        s.ok_runs.to_string(),
        s.failed_runs.to_string(),
        s.aborted_runs.to_string(),
        s.in_flight_runs.to_string(),
        s.active_sessions.to_string(),
    ];
    LABELS
        .iter()
        .zip(values)
        .map(|(label, value)| Band {
            label: (*label).to_string(),
            value,
        })
        .collect()
}

/// 运行终态分布。原位置是「按 Agent 的 Token 分布」——主副 agent 的 token
/// 归属在 ledger 里不存在（`agent_token_split` 在 `unavailable` 里），换成
/// 同样是「占比条」形态、但有真实数据的终态分布。
fn build_status_bars(s: Option<&StatsSummary>) -> Vec<StatusBar> {
    let rows: [(&str, u64, &str); 4] = match s {
        Some(s) => [
            ("成功", s.ok_runs, BRAND),
            ("失败", s.failed_runs, "#f0666f"),
            ("已中止", s.aborted_runs, "#e0a341"),
            ("进行中", s.in_flight_runs, "#4ed17e"),
        ],
        None => [
            ("成功", 0, BRAND),
            ("失败", 0, "#f0666f"),
            ("已中止", 0, "#e0a341"),
            ("进行中", 0, "#4ed17e"),
        ],
    };
    let total: u64 = rows.iter().map(|(_, n, _)| *n).sum();
    rows.iter()
        .map(|(label, n, color)| StatusBar {
            label: (*label).to_string(),
            count: n.to_string(),
            pct: if total > 0 {
                *n as f64 * 100.0 / total as f64
            } else {
                0.0
            },
            color: (*color).to_string(),
        })
        .collect()
}

/// 吞吐折线：每桶的运行数。空 summary → 空序列（图表画布仍渲染）。
fn build_points(s: Option<&StatsSummary>) -> Vec<Point> {
    s.map(|s| {
        s.throughput
            .iter()
            .map(|b| Point {
                value: b.runs as f64,
            })
            .collect()
    })
    .unwrap_or_default()
}

/// 最近运行 feed。model / provider / cost 在 ledger 里不存在，改显示
/// run id、session id、耗时、终态。
fn build_feed(s: Option<&StatsSummary>) -> Vec<Feed> {
    let Some(s) = s else {
        return Vec::new();
    };
    s.recent
        .iter()
        .map(|r| Feed {
            run_id: r.run_id.clone(),
            session_id: r.session_id.clone(),
            duration: match r.duration_ms {
                Some(ms) => format_duration(ms as f64),
                None => "进行中".to_string(),
            },
            status: r.status.clone(),
            status_class: match r.status.as_str() {
                "ok" => "text-success-2".to_string(),
                "running" => "text-brand".to_string(),
                // 兜底是错误色而非中性色：除 ok/running 外 daemon 的终态
                // （failed / aborted / spawn_failed / unknown）一律是非成功，
                // 按错误展示是对的。这里有意不做日志——build_feed 在渲染路径
                // 上每次重算都跑，未知态会刷屏；新增终态的兜底视觉就是错误色，
                // 不会造成「成功被显示成失败」的反向误判。
                _ => "text-danger".to_string(),
            },
        })
        .collect()
}

/// 「暂无数据源」说明行：把 daemon 报的 `unavailable` 键翻成中文短语。
/// 这样页面上缺失的卡片有据可查，而不是静默消失或填一个假零。
fn build_unavailable_note(s: Option<&StatsSummary>) -> String {
    let Some(s) = s else {
        return "未连接 daemon：统计数据不可用。".to_string();
    };
    if s.unavailable.is_empty() {
        return String::new();
    }
    let names: Vec<&str> = s
        .unavailable
        .iter()
        .map(|k| match k.as_str() {
            "tokens" => "token 用量",
            "cost" => "费用",
            "cache" => "缓存命中",
            "ttft" => "首字延迟",
            "agent_token_split" => "主副 agent token 分布",
            "model" => "模型",
            "provider" => "供应商",
            other => other,
        })
        .collect();
    format!("暂无数据源（需 token 计量）：{}。", names.join("、"))
}

// ── 小工具 ──────────────────────────────────────────────────────────────

/// 终态运行里的错误占比。分母只算已结束的 run（在飞 run 还没有结果，
/// 计入分母会让错误率被稀释）。无终态 run → `None`。
fn error_rate(s: &StatsSummary) -> Option<f64> {
    let terminal = s.ok_runs + s.failed_runs + s.aborted_runs;
    (terminal > 0).then(|| s.error_runs() as f64 * 100.0 / terminal as f64)
}

/// 计数类指标的环比 chip。`higher_is_good` 决定涨了算好还是坏；上一窗口
/// 缺失（All 范围）或为 0（没有可比基数）时给中性 `Flat`，不编百分比。
fn delta_count(cur: u64, prev: Option<u64>, higher_is_good: bool) -> DeltaChip {
    let (delta, tone) = match prev {
        Some(p) if p > 0 => {
            let pct = (cur as f64 - p as f64) * 100.0 / p as f64;
            (format!("{pct:+.1}%"), tone_for(pct, higher_is_good))
        }
        Some(_) => ("环比无基数".to_string(), Tone::Flat),
        None => ("无可比窗口".to_string(), Tone::Flat),
    };
    DeltaChip { delta, tone }
}

/// 平均耗时的环比 chip——耗时变短是好事。
fn delta_avg(cur: Option<f64>, prev: Option<f64>) -> DeltaChip {
    let (delta, tone) = match (cur, prev) {
        (Some(c), Some(p)) if p > 0.0 => {
            let pct = (c - p) * 100.0 / p;
            (format!("{pct:+.1}%"), tone_for(pct, false))
        }
        _ => ("无可比窗口".to_string(), Tone::Flat),
    };
    DeltaChip { delta, tone }
}

/// 变化率 → 色调。零变化恒中性。
fn tone_for(pct: f64, higher_is_good: bool) -> Tone {
    if pct.abs() < f64::EPSILON {
        return Tone::Flat;
    }
    if (pct > 0.0) == higher_is_good {
        Tone::Good
    } else {
        Tone::Bad
    }
}

/// 毫秒 → 人读串：`820ms` / `12.4s` / `3m12s`。
fn format_duration(ms: f64) -> String {
    if !ms.is_finite() || ms < 0.0 {
        return "—".to_string();
    }
    if ms < 1000.0 {
        return format!("{:.0}ms", ms);
    }
    let secs = ms / 1000.0;
    if secs < 60.0 {
        return format!("{secs:.1}s");
    }
    let mins = (secs / 60.0).floor();
    let rem = secs - mins * 60.0;
    format!("{mins:.0}m{rem:.0}s")
}
