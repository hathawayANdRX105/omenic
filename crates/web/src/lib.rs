//! Dioxus LiveView Web UI for omenic.
//!
//! Real-time WebSocket interactive interface:
//!   - `/`        Workspace (session sidebar + chat + task board)
//!   - `/stats`   Observability dashboard with time range filtering
//!   - `/config`  Model / channel configuration

pub mod components;
pub mod llm;
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
    let mut runtime_config = use_signal(llm::LlmRuntimeConfig::load_from_system);
    let mut show_quick_switcher = use_signal(|| false);
    let css_content = include_str!("../assets/main.css");

    rsx! {
        style { "{css_content}" }

        nav { class: "top-nav",
            div { class: "nav-left",
                div { class: "nav-workspace",
                    span { class: "workspace-title", "omenic" }
                    span { class: "workspace-sep", "/" }
                    span { class: "workspace-branch", "feat/web-agent-harness" }
                    span { class: "workspace-status", "clean" }
                }
            }
            div { class: "nav-center",
                button {
                    class: "nav-search-bar",
                    onclick: move |_| {
                        if current_tab() != Tab::Workspace {
                            current_tab.set(Tab::Workspace);
                        }
                        show_quick_switcher.set(true);
                    },
                    span { "搜索会话" }
                    kbd { "⌘K" }
                }
            }
            div { class: "nav-right",
                div { class: "nav-tabs",
                    button {
                        class: if current_tab() == Tab::Workspace { "nav-tab active" } else { "nav-tab" },
                        onclick: move |_| current_tab.set(Tab::Workspace),
                        "工作区"
                    }
                    button {
                        class: if current_tab() == Tab::Stats { "nav-tab active" } else { "nav-tab" },
                        onclick: move |_| current_tab.set(Tab::Stats),
                        "数据统计"
                    }
                    button {
                        class: if current_tab() == Tab::Config { "nav-tab active" } else { "nav-tab" },
                        onclick: move |_| current_tab.set(Tab::Config),
                        "配置"
                    }
                }
                span { class: "nav-version", "v0.1.0" }
            }
        }

        match current_tab() {
            Tab::Workspace => rsx! {
                Workspace {
                    config: runtime_config(),
                    on_update_config: move |cfg| runtime_config.set(cfg),
                    show_quick_switcher: show_quick_switcher,
                }
            },
            Tab::Stats => rsx! { Stats {} },
            Tab::Config => rsx! {
                ConfigPage {
                    config: runtime_config(),
                    on_update_config: move |cfg| runtime_config.set(cfg),
                }
            },
        }
    }
}

/// Launch the interactive Dioxus LiveView server on http://127.0.0.1:8080.
pub async fn launch() {
    let port = std::env::var("PORT")
        .or_else(|_| std::env::var("OMENIC_WEB_PORT"))
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(8026);
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
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
    <script>
    (function() {{
        function scrollToBottom() {{
            const chatEl = document.querySelector(".chat-messages");
            if (chatEl) {{
                chatEl.scrollTop = chatEl.scrollHeight;
            }}
        }}

        // Track IME composition explicitly: isComposing alone is unreliable on fcitx/ibus + Linux
        let composing = {{ active: false }};
        document.addEventListener("compositionstart", function(e) {{
            if (e.target && e.target.id === "chat-input-area") composing.active = true;
        }}, true);
        document.addEventListener("compositionend", function(e) {{
            if (e.target && e.target.id === "chat-input-area") composing.active = false;
        }}, true);

        // Handle Enter key on textarea to submit form
        document.addEventListener("keydown", function(e) {{
            if (e.target && e.target.id === "chat-input-area" && e.key === "Enter" && !e.shiftKey) {{
                if (composing.active || e.isComposing || e.keyCode === 229) return;
                e.preventDefault();
                const form = e.target.closest("form");
                if (form) {{
                    form.requestSubmit();
                    setTimeout(function() {{
                        e.target.value = "";
                    }}, 0);
                    setTimeout(scrollToBottom, 40);
                }}
            }}
        }}, true);

        // Clear textarea when clicking submit button (never mid-composition)
        document.addEventListener("click", function(e) {{
            const btn = e.target.closest("button[type='submit']");
            if (btn) {{
                if (composing.active) return;
                const form = btn.closest("form");
                if (form) {{
                    const ta = form.querySelector("textarea");
                    if (ta) {{
                        setTimeout(function() {{
                            ta.value = "";
                        }}, 0);
                    }}
                    setTimeout(scrollToBottom, 40);
                }}
            }}
        }}, true);

        // Global Cmd+K / Ctrl+K for quick switcher
        document.addEventListener("keydown", function(e) {{
            if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {{
                e.preventDefault();
                const btn = document.querySelector(".nav-search-bar");
                if (btn) btn.click();
            }}
        }});
        // Auto-scroll chat container ONLY when streaming and user is already near bottom
        let scrollTimeout = null;
        function isNearBottom(el) {{
            return el.scrollHeight - el.scrollTop - el.clientHeight < 360;
        }}
        const observer = new MutationObserver(function(mutations) {{
            const chatEl = document.querySelector(".chat-messages");
            if (!chatEl) return;
            for (let i = 0; i < mutations.length; i++) {{
                const target = mutations[i].target;
                if (target && target.closest && target.closest(".tool-accordion")) {{
                    return;
                }}
                if (mutations[i].addedNodes.length > 0 && isNearBottom(chatEl)) {{
                    clearTimeout(scrollTimeout);
                    scrollTimeout = setTimeout(scrollToBottom, 30);
                    break;
                }}
            }}
        }});
        setTimeout(scrollToBottom, 150);
        observer.observe(document.body, {{ childList: true, subtree: true }});
    }})();
    </script>
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

    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Failed to bind to {addr}: {e}");
            return;
        }
    };
    println!("omenic web server running on http://{addr}");
    let _ = axum::serve(listener, app.into_make_service()).await;
}
