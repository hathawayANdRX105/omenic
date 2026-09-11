use crate::mock::StatusLine;
use dioxus::prelude::*;

#[component]
pub fn StatuslineBar(statusline: StatusLine) -> Element {
    rsx! {
        div { class: "flex items-center gap-3 px-4 py-1.5 bg-surface border-t border-subtle font-mono text-[11px] text-muted-foreground",
            span { class: "text-foreground", "🧠 {statusline.model}" }
            span { class: "text-muted", "│" }
            span { class: "text-muted-foreground", "thinking: {statusline.thinking}" }
            span { class: "text-muted", "│" }
            span { class: "text-muted-foreground", "📊 {statusline.tokens_in}→{statusline.tokens_out}" }
            span { class: "text-muted", "│" }
            span { class: "text-muted-foreground", "💰 ${statusline.cost_usd:.3}" }
            span { class: "text-muted", "│" }
            div { class: "flex items-center gap-2",
                div { class: "w-24 h-1.5 rounded-full bg-base overflow-hidden",
                    div {
                        class: "h-full rounded-full bg-accent",
                        style: "width: {statusline.context_pct}%"
                    }
                }
                span { "context: {statusline.context_pct}%" }
            }
        }
    }
}
