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

use crate::components::ui::{Button, ButtonVariant};
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
    let css_content = format!(
        "{}",
        include_str!(concat!(env!("OUT_DIR"), "/tailwind.gen.css"))
    );

    rsx! {
        style { "{css_content}" }

        nav { class: "flex items-center h-[46px] px-4 bg-sidebar border-b border-subtle sticky top-0 z-50 select-none",
            // 左侧占位，保证中间按钮真正水平居中
            div { class: "flex-1" }
            // 中间：搜索会话
            div { class: "flex items-center",
                Button {
                    variant: ButtonVariant::Ghost,
                    class: "border border-subtle",
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
            // 右侧：标签 + 版本号（右对齐）
            div { class: "flex-1 flex items-center justify-end gap-2",
                div { class: "flex gap-0.5",
                    button {
                        class: if current_tab() == Tab::Workspace { "px-3 py-1.5 text-[12px] font-medium text-foreground bg-surface-elevated border-none rounded-md cursor-pointer transition-colors" } else { "px-3 py-1.5 text-[12px] font-medium text-muted-foreground bg-transparent border-none rounded-md cursor-pointer transition-colors hover:text-foreground" },
                        onclick: move |_| current_tab.set(Tab::Workspace),
                        "工作区"
                    }
                    button {
                        class: if current_tab() == Tab::Stats { "px-3 py-1.5 text-[12px] font-medium text-foreground bg-surface-elevated border-none rounded-md cursor-pointer transition-colors" } else { "px-3 py-1.5 text-[12px] font-medium text-muted-foreground bg-transparent border-none rounded-md cursor-pointer transition-colors hover:text-foreground" },
                        onclick: move |_| current_tab.set(Tab::Stats),
                        "数据统计"
                    }
                    button {
                        class: if current_tab() == Tab::Config { "px-3 py-1.5 text-[12px] font-medium text-foreground bg-surface-elevated border-none rounded-md cursor-pointer transition-colors" } else { "px-3 py-1.5 text-[12px] font-medium text-muted-foreground bg-transparent border-none rounded-md cursor-pointer transition-colors hover:text-foreground" },
                        onclick: move |_| current_tab.set(Tab::Config),
                        "配置"
                    }
                }
                span { class: "font-mono text-[11px] text-muted ml-2", "v0.1.0" }
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
    let css = format!(
        "{}",
        include_str!(concat!(env!("OUT_DIR"), "/tailwind.gen.css"))
    );

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

        function scrollToId(id) {{
            const el = document.getElementById(id);
            if (!el) return;
            el.scrollIntoView({{ behavior: "smooth", block: "start" }});
        }}

        // Minimap: click/wheel navigation + scroll-spy highlight, all client-side.
        // Bars carry a `data-anchor` (the prompt element id). A discrete 3-level
        // gradient is centered on a reference index (scroll position at rest, the
        // cursor while hovering): only the center bar + the two on each side (5 bars
        // total) are emphasized via LENGTH + BRIGHTNESS — center longest/brightest,
        // ±1 medium, ±2 short; every other bar stays one uniform width. No color fill.
        function setupMinimap() {{
            var mm = document.getElementById('minimap');
            if (!mm) return;
            var scrollEl = document.getElementById('chat-scroll');
            // Namespaced, persistent shared state (survives repeated setupMinimap calls
            // from the MutationObserver) to avoid polluting globals and stale caches.
            var NS = window.__mm = window.__mm || {{}};
            if (typeof NS.ref !== 'number') NS.ref = 0;
            // Cached DOM queries; invalidated whenever the chat DOM changes.
            function invalidate() {{ NS.barsCache = null; NS.centers = null; }}
            function getBars() {{
                if (!NS.barsCache) NS.barsCache = Array.prototype.slice.call(mm.querySelectorAll('[data-anchor]'));
                return NS.barsCache;
            }}
            function getCenters() {{
                if (!NS.centers) {{
                    var bs = getBars();
                    NS.centers = bs.map(function(b) {{
                        var r = b.getBoundingClientRect();
                        return r.top + r.height / 2;
                    }});
                }}
                return NS.centers;
            }}
            // 3-step prominence centered on index c: 0=center, 1=adjacent, 2=outer, else=uniform.
            function applyGradient(c) {{
                NS.ref = c;
                var bs = getBars();
                for (var i = 0; i < bs.length; i++) {{
                    var bar = bs[i].querySelector('.minimap-bar');
                    if (!bar) continue;
                    var off = Math.abs(i - c);
                    var width, op;
                    if (off === 0)      {{ width = 28; op = 1.0; }}
                    else if (off === 1) {{ width = 20; op = 0.72; }}
                    else if (off === 2) {{ width = 14; op = 0.5; }}
                    else                {{ width = 10; op = 0.34; }}
                    bar.style.width = width + 'px';
                    bar.style.opacity = op.toFixed(3);
                    bar.classList.toggle('bg-accent', off === 0);
                    bar.classList.toggle('bg-subtle', off !== 0);
                }}
            }}
            function nearestIndex(y) {{
                var cs = getCenters();
                var best = 0, bestD = Infinity;
                for (var i = 0; i < cs.length; i++) {{
                    var d = Math.abs(y - cs[i]);
                    if (d < bestD) {{ bestD = d; best = i; }}
                }}
                return best;
            }}
            function scrollToIdx(idx) {{
                var bs = getBars();
                if (idx < 0 || idx >= bs.length) return;
                var anchor = bs[idx].getAttribute('data-anchor');
                if (anchor) scrollToId(anchor);
                applyGradient(idx);
            }}
            // Bottom detection with a tolerance instead of a brittle -4 magic number.
            var BOTTOM_TOLERANCE = 32;
            function isAtBottom() {{
                return scrollEl.scrollHeight - scrollEl.scrollTop - scrollEl.clientHeight <= BOTTOM_TOLERANCE;
            }}
            function scrollIndex() {{
                var bs = getBars();
                if (!bs.length) return 0;
                if (isAtBottom()) return bs.length - 1;
                var top = scrollEl.getBoundingClientRect().top;
                var active = 0;
                for (var i = 0; i < bs.length; i++) {{
                    var a = bs[i].getAttribute('data-anchor');
                    var el = a ? document.getElementById(a) : null;
                    if (!el) continue;
                    if (el.getBoundingClientRect().top - top <= 120) active = i;
                }}
                return active;
            }}
            // rAF-throttled paint: hot paths (mousemove/scroll) write the DOM at most
            // once per frame, and only when the target index actually changes. This
            // avoids layout thrashing from per-event getBoundingClientRect reads.
            var rafPending = false;
            var pending = null;
            var lastPainted = null;
            function schedule(c) {{
                if (c === lastPainted && pending === null) return;
                pending = c;
                if (rafPending) return;
                rafPending = true;
                requestAnimationFrame(function() {{
                    rafPending = false;
                    if (pending === null) return;
                    applyGradient(pending);
                    lastPainted = pending;
                    pending = null;
                }});
            }}
            if (!NS.wired) {{
                NS.wired = true;
                mm.addEventListener('click', function(e) {{
                    var bar = e.target.closest('[data-anchor]');
                    if (!bar) return;
                    scrollToIdx(getBars().indexOf(bar));
                }});
                mm.addEventListener('wheel', function(e) {{
                    e.preventDefault();
                    var bs = getBars();
                    if (!bs.length) return;
                    var cur = (typeof NS.ref === 'number') ? NS.ref : scrollIndex();
                    var dir = e.deltaY > 0 ? 1 : -1;
                    var nxt = Math.max(0, Math.min(bs.length - 1, cur + dir));
                    scrollToIdx(nxt);
                }}, {{ passive: false }});
                mm.addEventListener('mouseover', function(e) {{
                    var bar = e.target.closest('[data-anchor]');
                    if (!bar) return;
                    var tip = bar.parentElement.querySelector('[data-tip]');
                    if (tip) tip.style.display = 'block';
                }});
                mm.addEventListener('mouseout', function(e) {{
                    var bar = e.target.closest('[data-anchor]');
                    if (!bar) return;
                    var to = e.relatedTarget;
                    if (to && bar.contains(to)) return;
                    var tip = bar.parentElement.querySelector('[data-tip]');
                    if (tip) tip.style.display = 'none';
                }});
                // Cache bar centers when the hover begins; subsequent snapping reads the
                // cache (no per-event getBoundingClientRect → no layout thrashing).
                mm.addEventListener('mouseenter', function() {{ NS.centers = null; getCenters(); }});
                mm.addEventListener('mousemove', function(e) {{ schedule(nearestIndex(e.clientY)); }});
                mm.addEventListener('mouseleave', function() {{ schedule(scrollIndex()); }});
                if (scrollEl) {{
                    scrollEl.addEventListener('scroll', function() {{
                        NS.centers = null; // chat scrolled; cached hover centers are stale
                        schedule(scrollIndex());
                    }});
                    window.addEventListener('resize', function() {{ invalidate(); }});
                }}
            }}
            // DOM may have changed (new messages): re-query and repaint from scroll position.
            invalidate();
            var init = scrollIndex();
            lastPainted = init;
            applyGradient(init);
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
        // Smart scroll pin: 用户主动向上滚动后解除跟随,回到底部附近才恢复
        const PIN_EPS = 48;
        const pinned = {{ value: true }};
        function markPinned(el) {{
            if (!el) return;
            pinned.value = el.scrollHeight - el.scrollTop - el.clientHeight < PIN_EPS;
        }}
        function setUpPin() {{
            const chatEl = document.querySelector(".chat-messages");
            if (chatEl && !chatEl.__pinWired) {{
                chatEl.__pinWired = true;
                chatEl.addEventListener("scroll", function() {{ markPinned(chatEl); }});
            }}
        }}
        let scrollTimeout = null;
        const observer = new MutationObserver(function(mutations) {{
            setupMinimap();
            const chatEl = document.querySelector(".chat-messages");
            if (!chatEl) return;
            let insideTools = false;
            for (let i = 0; i < mutations.length; i++) {{
                const target = mutations[i].target;
                if (target && target.closest && target.closest(".tool-accordion")) {{
                    insideTools = true;
                    break;
                }}
            }}
            if (insideTools) return;
            setUpPin();
            if (pinned.value) {{
                clearTimeout(scrollTimeout);
                scrollTimeout = setTimeout(scrollToBottom, 30);
            }}
        }});
        setUpPin();
        setTimeout(scrollToBottom, 150);
        observer.observe(document.body, {{ childList: true, subtree: true }});
        setupMinimap();
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
    let _ = axum::serve(listener, app).await;
}
