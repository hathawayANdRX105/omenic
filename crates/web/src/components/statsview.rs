use crate::mock::*;
use dioxus::prelude::*;

#[component]
pub fn StatsView() -> Element {
    let mut selected_range = use_signal(|| "24h".to_string());
    let data = mock_stats_for_range(&selected_range());
    let ranges = ["1h", "24h", "7d", "30d", "90d", "All"];

    rsx! {
        div { class: "flex-1 overflow-y-auto flex flex-col gap-6 px-12 pt-8 pb-14 max-w-[1400px] w-full mx-auto",
            // Header
            div { class: "flex items-center justify-between pb-5 border-b border-subtle",
                div {
                    h1 { class: "text-[22px] font-bold text-white tracking-tight m-0", "数据统计" }
                    p { class: "text-[13px] text-muted mt-1 m-0", "当前视图范围：最近 {selected_range()}" }
                }
                div { class: "flex gap-0.5 bg-[rgba(255,255,255,0.03)] border border-subtle rounded-lg p-0.5",
                    for r in ranges {
                        {
                            let r_str = r.to_string();
                            rsx! {
                                button {
                                    key: "{r}",
                                    class: if selected_range() == r { "px-3.5 py-1 text-xs rounded-md font-semibold bg-surface-elevated text-white shadow-sm" } else { "px-3.5 py-1 text-xs rounded-md text-muted hover:text-primary transition-colors" },
                                    onclick: move |_| selected_range.set(r_str.clone()),
                                    "{r}"
                                }
                            }
                        }
                    }
                }
            }

            // KPI Cards
            div { class: "grid grid-cols-5 gap-3.5",
                for kpi in &data.kpis {
                    KpiCardView { key: "{kpi.label}", kpi: kpi.clone() }
                }
            }

            // Sub-metrics ribbon
            div { class: "grid grid-cols-7 gap-3 bg-[rgba(255,255,255,0.015)] border border-subtle rounded-lg px-4.5 py-3.5",
                for sm in &data.sub_metrics {
                    div { key: "{sm.label}", class: "flex flex-col gap-1",
                        div { class: "text-[10px] font-semibold text-muted font-mono uppercase tracking-wider", "{sm.label}" }
                        div { class: "text-[15px] font-semibold text-[#e2e4ed] font-mono", "{sm.value}" }
                    }
                }
            }

            // Body: 3-column
            div { class: "grid grid-cols-[320px_1fr_340px] gap-4 items-stretch",
                div { class: "bg-surface border border-subtle rounded-[10px] p-5 flex flex-col gap-4",
                    h3 { class: "text-sm font-semibold text-white m-0", "按 Agent 的 Token 分布" }
                    for bar in &data.agent_bars {
                        AgentBarRow { key: "{bar.agent}", bar: bar.clone() }
                    }
                }

                div { class: "bg-surface border border-subtle rounded-[10px] p-5 flex flex-col gap-4",
                    h3 { class: "text-sm font-semibold text-white m-0", "吞吐趋势 ({selected_range()})" }
                    ThroughputChart { points: data.throughput.clone() }
                }

                div { class: "bg-surface border border-subtle rounded-[10px] p-5 flex flex-col gap-4",
                    h3 { class: "text-sm font-semibold text-white m-0", "最近请求 Feed" }
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
        "bg-emerald-500/10 text-emerald-400 border border-emerald-500/25"
    } else {
        "bg-red-500/10 text-red-400 border border-red-500/25"
    };
    rsx! {
        div { class: "bg-surface border border-subtle rounded-[10px] px-4 py-4 flex flex-col gap-2 hover:border-hover hover:-translate-y-px transition-all",
            div { class: "text-xs text-secondary font-medium", "{kpi.label}" }
            div { class: "text-[26px] font-bold text-white font-mono leading-none", "{kpi.value}" }
            span { class: "{delta_class} text-[11px] font-semibold px-2 py-0.5 rounded w-fit font-mono", "{kpi.delta}" }
        }
    }
}

#[component]
fn AgentBarRow(bar: AgentTokenBar) -> Element {
    rsx! {
        div { class: "flex flex-col gap-2 px-2.5 py-2 bg-base border border-subtle rounded-md",
            div { class: "flex items-center justify-between text-xs",
                span { class: "font-semibold text-white", "{bar.agent}" }
                div { class: "flex items-center gap-2",
                    span { class: "text-muted font-mono text-[11px]", "{bar.tokens}" }
                    span { class: "font-semibold text-accent font-mono text-[11px]", "{bar.pct:.1}%" }
                }
            }
            div { class: "h-1.5 bg-[rgba(255,255,255,0.08)] rounded-full overflow-hidden",
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
                stroke: "#a28ac7",
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
                            fill: "#a28ac7",
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
        div { class: "flex items-center justify-between px-3 py-2 bg-base border border-subtle rounded-md text-xs hover:border-hover hover:bg-hover transition-colors",
            div { class: "flex items-center gap-2",
                span { class: "font-semibold text-[#c4b5fd] font-mono text-[11px]", "{item.model}" }
                span { class: "text-[10px] text-muted bg-[rgba(255,255,255,0.05)] px-1.5 py-px rounded-sm", "{item.provider}" }
            }
            div { class: "flex items-center gap-2.5 font-mono text-[11px]",
                span { class: "text-muted text-[10px]", "{item.time_ago}" }
                span { class: "text-muted", "{item.duration}" }
                span { class: "text-emerald-400 font-medium", "{item.cost}" }
            }
        }
    }
}
