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
    let css_content = include_str!("../assets/main.css");

    rsx! {
        // Direct inline style injection to ensure zero desync / white browser defaults
        style { "{css_content}" }

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
                div { class: "nav-status-badge",
                    span { class: "dot" }
                    span { "{runtime_config().model}" }
                }
                span { class: "nav-version", "v0.1.0" }
            }
        }

        match current_tab() {
            Tab::Workspace => rsx! {
                Workspace {
                    config: runtime_config(),
                    on_update_config: move |cfg| runtime_config.set(cfg),
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
    <script>
    // Client-side instant input clear & auto-scroll watchdog
    (function() {{
        function scrollToBottom() {{
            const chatEl = document.querySelector(".chat-messages");
            if (chatEl) {{
                chatEl.scrollTop = chatEl.scrollHeight + 5000;
            }}
            const anchor = document.getElementById("chat-scroll-anchor");
            if (anchor) {{
                anchor.scrollIntoView({{ behavior: "instant", block: "end" }});
            }}
        }}

        function clearInput() {{
            const ta = document.getElementById("chat-input-area") || document.querySelector(".chat-input-field");
            if (ta) {{
                ta.value = "";
                ta.textContent = "";
                ta.dispatchEvent(new Event("input", {{ bubbles: true }}));
                [10, 30, 80, 150, 300].forEach(function(delay) {{
                    setTimeout(function() {{
                        if (ta) {{
                            ta.value = "";
                        }}
                    }}, delay);
                }});
            }}
        }}

        // 1. Synchronously handle Enter key on textarea
        document.addEventListener("keydown", function(e) {{
            const isInput = e.target && (e.target.id === "chat-input-area" || e.target.classList.contains("chat-input-field"));
            if (isInput && e.key === "Enter" && !e.shiftKey) {{
                if (e.isComposing || e.keyCode === 229) {{
                    return; // Do not intercept during IME composition
                }}
                e.preventDefault(); // Stop trailing newline
                clearInput();
                scrollToBottom();
                setTimeout(scrollToBottom, 50);
                setTimeout(scrollToBottom, 200);
            }}
        }}, true);

        // 2. Clear input and scroll on send button click
        document.addEventListener("click", function(e) {{
            const btn = e.target.closest(".btn-send-round");
            if (btn) {{
                clearInput();
                scrollToBottom();
                setTimeout(scrollToBottom, 50);
                setTimeout(scrollToBottom, 200);
            }}
        }}, true);

        // 3. Auto-scroll chat container whenever new messages arrive
        let scrollTimeout = null;
        const observer = new MutationObserver(function() {{
            clearTimeout(scrollTimeout);
            scrollTimeout = setTimeout(scrollToBottom, 20);
        }});
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

    println!("⚡ omenic web server running on http://{addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}
