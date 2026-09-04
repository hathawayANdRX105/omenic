//! Dioxus LiveView Web UI for omenic.
//!
//! Real-time WebSocket interactive interface:
//!   - `/`        Workspace (session sidebar + chat + task board)
//!   - `/stats`   Observability dashboard with time range filtering
//!   - `/config`  Model / channel configuration

pub mod components;
pub mod mock;
pub mod pages;

use dioxus::prelude::*;
use pages::config_page::ConfigPage;
use pages::stats::Stats;
use pages::workspace::Workspace;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Workspace,
    Stats,
    Config,
}

#[component]
pub fn App() -> Element {
    let mut current_tab = use_signal(|| Tab::Workspace);

    rsx! {
        nav { class: "top-nav",
            div { class: "nav-brand",
                span { class: "nav-logo", "⚡" }
                span { class: "nav-title", "omenic" }
            }
            div { class: "nav-tabs",
                button {
                    class: if current_tab() == Tab::Workspace { "nav-tab nav-tab-active" } else { "nav-tab" },
                    onclick: move |_| current_tab.set(Tab::Workspace),
                    "工作区"
                }
                button {
                    class: if current_tab() == Tab::Stats { "nav-tab nav-tab-active" } else { "nav-tab" },
                    onclick: move |_| current_tab.set(Tab::Stats),
                    "数据统计"
                }
                button {
                    class: if current_tab() == Tab::Config { "nav-tab nav-tab-active" } else { "nav-tab" },
                    onclick: move |_| current_tab.set(Tab::Config),
                    "配置"
                }
            }
            div { class: "nav-right",
                span { class: "nav-version", "v0.1.0" }
            }
        }

        match current_tab() {
            Tab::Workspace => rsx! { Workspace {} },
            Tab::Stats => rsx! { Stats {} },
            Tab::Config => rsx! { ConfigPage {} },
        }
    }
}

/// Launch the interactive Dioxus LiveView server on http://127.0.0.1:8080.
pub async fn launch() {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 8080));
    let view = dioxus_liveview::LiveViewPool::new();
    let glue = dioxus_liveview::interpreter_glue("/ws");
    let css = include_str!("../assets/main.css");

    let index_html = format!(
        r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>omenic</title>
    <style>
{css}
    </style>
</head>
<body>
    <div id="main"></div>
    {glue}
</body>
</html>"#
    );

    let app = axum::Router::new()
        .route(
            "/ws",
            axum::routing::get(move |ws: axum::extract::WebSocketUpgrade| async move {
                ws.on_upgrade(move |socket| async move {
                    _ = view
                        .launch_virtualdom(dioxus_liveview::axum_socket(socket), move || {
                            VirtualDom::new(App)
                        })
                        .await;
                })
            }),
        )
        .fallback(axum::routing::get(move || async move {
            axum::response::Html(index_html.clone())
        }));

    println!("⚡ omenic web server running on http://{addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}
