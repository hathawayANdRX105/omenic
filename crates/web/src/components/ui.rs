//! Reusable, style-unified UI primitives.
//!
//! These centralize the Tailwind class strings that were previously inlined all
//! over the workspace/chat components, so button/panel styling stays consistent
//! (and easy to retheme). They are the local stand-in for a shared component
//! library: the `dioxus-components` crate can't be pulled into this sandbox
//! (no outbound network) and its utility classes wouldn't be picked up by our
//! Tailwind scan anyway, so we keep a single source of truth here.

use dioxus::prelude::*;

#[derive(Clone, PartialEq, Default)]
pub enum ButtonVariant {
    #[default]
    Primary,
    Ghost,
    Subtle,
    Danger,
}

impl ButtonVariant {
    fn classes(&self) -> &'static str {
        match self {
            // Accent fill — the "primary action" look the user liked on the old
            // Spaces tab, generalized into a real button.
            ButtonVariant::Primary => {
                "bg-accent text-[#0a0a0d] hover:bg-accent-hover border border-transparent"
            }
            ButtonVariant::Ghost => {
                "bg-transparent border border-subtle text-muted hover:text-primary hover:border-hover hover:bg-hover"
            }
            ButtonVariant::Subtle => {
                "bg-surface text-primary hover:bg-hover border border-subtle"
            }
            ButtonVariant::Danger => {
                "bg-transparent border border-subtle text-muted hover:text-danger hover:border-[rgba(248,113,113,0.35)] hover:bg-[rgba(248,113,113,0.1)]"
            }
        }
    }
}

const BUTTON_BASE: &str =
    "inline-flex items-center justify-center gap-1.5 rounded-md text-[13px] font-medium px-3 py-1.5 transition-colors cursor-pointer select-none whitespace-nowrap";

#[component]
pub fn Button(
    children: Element,
    onclick: Option<EventHandler<MouseEvent>>,
    variant: Option<ButtonVariant>,
    title: Option<String>,
    class: Option<String>,
) -> Element {
    let v = variant.unwrap_or_default();
    let extra = class.unwrap_or_default();
    let cls = format!("{} {} {}", BUTTON_BASE, v.classes(), extra);
    rsx! {
        button {
            r#type: "button",
            class: "{cls}",
            title: title.unwrap_or_default(),
            onclick: move |evt| {
                if let Some(cb) = onclick {
                    cb.call(evt);
                }
            },
            {children}
        }
    }
}

const ICON_BASE: &str =
    "inline-flex items-center justify-center rounded-md transition-colors cursor-pointer select-none";

#[component]
pub fn IconButton(
    children: Element,
    onclick: Option<EventHandler<MouseEvent>>,
    title: String,
    variant: Option<ButtonVariant>,
    class: Option<String>,
) -> Element {
    let v = variant.unwrap_or(ButtonVariant::Ghost);
    let extra = class.unwrap_or_default();
    // p-1.5 keeps the ~28px hit target the inline icon buttons already had.
    let cls = format!("{} p-1.5 {} {}", ICON_BASE, v.classes(), extra);
    rsx! {
        button {
            r#type: "button",
            class: "{cls}",
            title: "{title}",
            onclick: move |evt| {
                if let Some(cb) = onclick {
                    cb.call(evt);
                }
            },
            {children}
        }
    }
}
