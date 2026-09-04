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

/// Launch the Dioxus application.
pub fn launch() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    rsx! {
        Router::<Route> {}
    }
}
