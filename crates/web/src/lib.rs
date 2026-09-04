//! Dioxus fullstack Web UI for omenic.
//!
//! Pages:
//!   - `/`        Workspace (session sidebar + chat + task board)
//!   - `/stats`   Observability dashboard
//!   - `/config`  Model / channel configuration

pub mod components;
pub mod mock;
pub mod pages;

use dioxus::prelude::*;
use pages::config_page::ConfigPage;
use pages::stats::Stats;
use pages::workspace::Workspace;

const MAIN_CSS: Asset = asset!("/assets/main.css");

#[derive(Debug, Clone, Routable, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    #[layout(Navbar)]
        #[route("/")]
        Workspace {},
        #[route("/stats")]
        Stats {},
        #[route("/config")]
        ConfigPage {},
    #[end_layout]
    #[route("/:..route")]
    NotFound { route: Vec<String> },
}

#[component]
fn Navbar() -> Element {
    rsx! {
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        nav { class: "top-nav",
            div { class: "nav-brand",
                span { class: "nav-logo", "⚡" }
                span { class: "nav-title", "omenic" }
            }
            div { class: "nav-tabs",
                Link {
                    class: "nav-tab",
                    active_class: "nav-tab-active",
                    to: Route::Workspace {},
                    "工作区"
                }
                Link {
                    class: "nav-tab",
                    active_class: "nav-tab-active",
                    to: Route::Stats {},
                    "数据统计"
                }
                Link {
                    class: "nav-tab",
                    active_class: "nav-tab-active",
                    to: Route::ConfigPage {},
                    "配置"
                }
            }
            div { class: "nav-right",
                span { class: "nav-version", "v0.1.0" }
            }
        }
        Outlet::<Route> {}
    }
}

#[component]
fn NotFound(route: Vec<String>) -> Element {
    rsx! {
        div { class: "not-found",
            h1 { "404" }
            p { "路径 /{route.join(\"/\")} 不存在" }
            Link { to: Route::Workspace {}, "返回工作区" }
        }
    }
}

/// Default fallback index.html template used in server-only / direct execution mode
const DEFAULT_INDEX_HTML: &str = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>omenic</title>
    <style>
"#;

const DEFAULT_INDEX_HTML_TAIL: &str = r#"
    </style>
</head>
<body>
    <div id="main"></div>
</body>
</html>"#;

/// Launch the Dioxus application.
pub fn launch() {
    #[cfg(feature = "server")]
    {
        let css = include_str!("../assets/main.css");
        let html = format!("{DEFAULT_INDEX_HTML}{css}{DEFAULT_INDEX_HTML_TAIL}");
        let cfg = dioxus::fullstack::ServeConfig::builder()
            .index_html(html)
            .build()
            .expect("valid index html");
        dioxus::LaunchBuilder::new().with_cfg(cfg).launch(App);
    }
    #[cfg(not(feature = "server"))]
    {
        dioxus::launch(App);
    }
}

#[component]
pub fn App() -> Element {
    rsx! {
        Router::<Route> {}
    }
}
