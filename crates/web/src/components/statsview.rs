use crate::mock::*;
use dioxus::prelude::*;

#[component]
pub fn StatsView() -> Element {
    let mut selected_range = use_signal(|| "24h".to_string());
    let data = mock_stats_for_range(&selected_range());
    let ranges = ["1h", "24h", "7d", "30d", "90d", "All"];

    rsx! {
        div { class: "stats-page",
            div { class: "stats-header",
                div {
                    h1 { "数据统计" }
                    p { class: "stats-subtitle", "当前视图范围：最近 {selected_range()}" }
                }
                div { class: "time-filter",
                    for r in ranges {
                        {
                            let r_str = r.to_string();
                            rsx! {
                                button {
                                    key: "{r}",
                                    class: if selected_range() == r { "time-btn active" } else { "time-btn" },
                                    onclick: move |_| selected_range.set(r_str.clone()),
                                    "{r}"
                                }
                            }
                        }
                    }
                }
            }

            // KPI Cards
            div { class: "kpi-row",
                for kpi in &data.kpis {
                    KpiCardView { key: "{kpi.label}", kpi: kpi.clone() }
                }
            }

            // Sub-metrics
            div { class: "sub-metrics",
                for sm in &data.sub_metrics {
                    div { key: "{sm.label}", class: "sub-metric",
                        div { class: "sub-metric-label", "{sm.label}" }
                        div { class: "sub-metric-value", "{sm.value}" }
                    }
                }
            }

            // Main content: bars + chart + feed
            div { class: "stats-body",
                // Left: agent token distribution
                div { class: "agent-bar-section",
                    h3 { "按 Agent 的 Token 分布" }
                    for bar in &data.agent_bars {
                        AgentBarRow { key: "{bar.agent}", bar: bar.clone() }
                    }
                }

                // Center: throughput chart
                div { class: "throughput-section",
                    h3 { "吞吐趋势 ({selected_range()})" }
                    ThroughputChart { points: data.throughput.clone() }
                }

                // Right: operational feed
                div { class: "feed-section",
                    h3 { "最近请求 Feed" }
                    div { class: "feed-list",
                        for item in &data.feed {
                            FeedRow { key: "{item.time_ago}", item: item.clone() }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn KpiCardView(kpi: KpiCard) -> Element {
    let delta_class = if kpi.delta_positive {
        "kpi-delta positive"
    } else {
        "kpi-delta negative"
    };
    rsx! {
        div { class: "kpi-card",
            div { class: "kpi-label", "{kpi.label}" }
            div { class: "kpi-value", "{kpi.value}" }
            span { class: "{delta_class}", "{kpi.delta}" }
        }
    }
}

#[component]
fn AgentBarRow(bar: AgentTokenBar) -> Element {
    rsx! {
        div { class: "agent-bar-row",
            div { class: "agent-bar-header",
                span { class: "agent-name", "{bar.agent}" }
                span { class: "agent-tokens", "{bar.tokens}" }
                span { class: "agent-pct", "{bar.pct:.1}%" }
            }
            div { class: "agent-bar-bg",
                div {
                    class: "agent-bar-fill",
                    style: "width: {bar.pct}%; background: {bar.color}",
                }
            }
        }
    }
}

#[component]
fn ThroughputChart(points: Vec<ThroughputPoint>) -> Element {
    let max_val = points
        .iter()
        .map(|p| p.tokens)
        .fold(f64::MIN, f64::max)
        .max(1.0);
    let width = 400.0_f64;
    let height = 180.0_f64;
    let pad = 20.0;
    let step = (width - pad * 2.0) / (points.len().max(2) - 1) as f64;

    let path_data: String = points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let x = pad + i as f64 * step;
            let y = height - pad - (p.tokens / max_val) * (height - pad * 2.0);
            if i == 0 {
                format!("M {x:.1} {y:.1}")
            } else {
                format!("L {x:.1} {y:.1}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    let area_data = format!(
        "{path_data} L {:.1} {:.1} L {:.1} {:.1} Z",
        pad + (points.len().max(2) - 1) as f64 * step,
        height - pad,
        pad,
        height - pad,
    );

    rsx! {
        svg {
            class: "throughput-chart",
            view_box: "0 0 {width} {height}",
            path {
                class: "chart-area",
                d: "{area_data}",
                fill: "url(#chartGrad)",
                opacity: "0.3",
            }
            path {
                class: "chart-line",
                d: "{path_data}",
                fill: "none",
                stroke: "#e91e63",
                stroke_width: "2",
            }
            defs {
                linearGradient {
                    id: "chartGrad",
                    x1: "0",
                    y1: "0",
                    x2: "0",
                    y2: "1",
                    stop { offset: "0%", stop_color: "#e91e63", stop_opacity: "0.4" }
                    stop { offset: "100%", stop_color: "#e91e63", stop_opacity: "0.0" }
                }
            }
            for (i, p) in points.iter().enumerate() {
                {
                    let x = pad + i as f64 * step;
                    let y = height - pad - (p.tokens / max_val) * (height - pad * 2.0);
                    rsx! {
                        circle {
                            key: "pt-{i}",
                            cx: "{x:.1}",
                            cy: "{y:.1}",
                            r: "3",
                            fill: "#e91e63",
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn FeedRow(item: FeedItem) -> Element {
    rsx! {
        div { class: "feed-item",
            div { class: "feed-main",
                span { class: "feed-model", "{item.model}" }
                span { class: "feed-provider", "{item.provider}" }
            }
            div { class: "feed-meta",
                span { class: "feed-time", "{item.time_ago}" }
                span { class: "feed-duration", "{item.duration}" }
                span { class: "feed-cost", "{item.cost}" }
            }
        }
    }
}
