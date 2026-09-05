use crate::mock::StatusLine;
use dioxus::prelude::*;

#[component]
pub fn StatuslineBar(statusline: StatusLine) -> Element {
    rsx! {
        div { class: "statusline",
            span { class: "statusline-model", "🧠 {statusline.model}" }
            span { class: "statusline-sep", "│" }
            span { class: "statusline-thinking", "thinking: {statusline.thinking}" }
            span { class: "statusline-sep", "│" }
            span { class: "statusline-tokens", "📊 {statusline.tokens_in}→{statusline.tokens_out}" }
            span { class: "statusline-sep", "│" }
            span { class: "statusline-cost", "💰 ${statusline.cost_usd:.3}" }
            span { class: "statusline-sep", "│" }
            div { class: "statusline-context",
                div { class: "context-bar-bg",
                    div {
                        class: "context-bar-fill",
                        style: "width: {statusline.context_pct}%"
                    }
                }
                span { "context: {statusline.context_pct}%" }
            }
        }
    }
}
