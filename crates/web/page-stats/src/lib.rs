//! 数据统计页（dsh 设计语言重刷）：KPI 卡、指标带、三列主体。

use dioxus::prelude::*;
use omenic_web_state::types::{AgentTokenBar, FeedItem, KpiCard, ThroughputPoint};

#[component]
pub fn StatsView() -> Element {
    let mut selected_range = use_signal(|| "24h".to_string());
    let data = omenic_web_mock::store::stats_for_range(&selected_range());
    let ranges = ["1h", "24h", "7d", "30d", "90d", "All"];

    rsx! {
        div { class: "flex-1 overflow-y-auto flex flex-col gap-6 px-10 pt-8 pb-14 max-w-[1200px] w-full mx-auto",
            // 头部
            div { class: "flex items-center justify-between pb-5 border-b border-b1",
                div {
                    h1 { class: "text-[22px] leading-7 font-semibold text-label tracking-tight m-0", "数据统计" }
                    p { class: "text-[13px] leading-5 text-label-3 mt-1 m-0", "当前视图范围：最近 {selected_range()}" }
                }
                div { class: "flex gap-0.5 bg-layer-1 border border-b1 rounded-[10px] p-0.5",
                    for r in ranges {
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

            // KPI 卡
            div { class: "grid grid-cols-5 gap-3.5",
                for kpi in &data.kpis {
                    KpiCardView { key: "{kpi.label}", kpi: kpi.clone() }
                }
            }

            // 指标带
            div { class: "grid grid-cols-7 gap-3 bg-layer-1 border border-b1 rounded-xl px-5 py-3.5",
                for sm in &data.sub_metrics {
                    div { key: "{sm.label}", class: "flex flex-col gap-1",
                        div { class: "text-[10px] leading-4 font-semibold text-caption font-mono uppercase tracking-wider", "{sm.label}" }
                        div { class: "text-[15px] leading-5 font-semibold text-label font-mono", "{sm.value}" }
                    }
                }
            }

            // 主体三列
            div { class: "grid grid-cols-[320px_1fr_340px] gap-4 items-stretch",
                div { class: "bg-layer-1 border border-b1 rounded-xl p-5 flex flex-col gap-4",
                    h3 { class: "text-[14px] leading-5 font-medium text-label m-0", "按 Agent 的 Token 分布" }
                    for bar in &data.agent_bars {
                        AgentBarRow { key: "{bar.agent}", bar: bar.clone() }
                    }
                }

                div { class: "bg-layer-1 border border-b1 rounded-xl p-5 flex flex-col gap-4",
                    h3 { class: "text-[14px] leading-5 font-medium text-label m-0", "吞吐趋势 ({selected_range()})" }
                    ThroughputChart { points: data.throughput.clone() }
                }

                div { class: "bg-layer-1 border border-b1 rounded-xl p-5 flex flex-col gap-4",
                    h3 { class: "text-[14px] leading-5 font-medium text-label m-0", "最近请求 Feed" }
                    div { class: "flex flex-col gap-2 overflow-y-auto max-h-[320px]",
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
        "bg-chip-success text-success-2"
    } else {
        "bg-chip-danger text-danger"
    };
    rsx! {
        div { class: "bg-layer-1 border border-b1 rounded-xl px-4 py-4 flex flex-col gap-2 hover:border-b2 transition-colors",
            div { class: "text-[12px] leading-5 text-label-3 font-medium", "{kpi.label}" }
            div { class: "text-[26px] leading-7 font-semibold text-label font-mono", "{kpi.value}" }
            span { class: "{delta_class} text-[11px] leading-4 font-semibold px-2 py-0.5 rounded-md w-fit font-mono", "{kpi.delta}" }
        }
    }
}

#[component]
fn AgentBarRow(bar: AgentTokenBar) -> Element {
    rsx! {
        div { class: "flex flex-col gap-2 px-2.5 py-2 bg-base border border-b1 rounded-lg",
            div { class: "flex items-center justify-between text-[12px] leading-5",
                span { class: "font-medium text-label", "{bar.agent}" }
                div { class: "flex items-center gap-2",
                    span { class: "text-label-3 font-mono text-[11px]", "{bar.tokens}" }
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
fn ThroughputChart(points: Vec<ThroughputPoint>) -> Element {
    let max_val = points
        .iter()
        .map(|p| p.tokens)
        .fold(f64::MIN, f64::max)
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
            let y = height - pad - (p.tokens / max_val) * (height - pad * 2.0);
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
                    let y = height - pad - (p.tokens / max_val) * (height - pad * 2.0);
                    rsx! {
                        circle {
                            key: "pt-{i}",
                            cx: "{x:.1}",
                            cy: "{y:.1}",
                            r: "2.5",
                            fill: "#679efe",
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
        div { class: "flex items-center justify-between px-3 py-2 bg-base border border-b1 rounded-lg text-[12px] hover:bg-ihover transition-colors",
            div { class: "flex items-center gap-2",
                span { class: "font-medium text-brand-300 font-mono text-[11px]", "{item.model}" }
                span { class: "text-[10px] text-label-3 bg-layer-2 px-1.5 py-px rounded-md", "{item.provider}" }
            }
            div { class: "flex items-center gap-2.5 font-mono text-[11px]",
                span { class: "text-[10px] text-caption", "{item.time_ago}" }
                span { class: "text-label-3", "{item.duration}" }
                span { class: "text-success-2 font-medium", "{item.cost}" }
            }
        }
    }
}
