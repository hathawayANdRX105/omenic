//! Reusable, style-unified UI primitives.
//!
//! Thin wrappers over `dioxus_components` (the Dioxus 0.7 + Tailwind v4 component
//! library) so the whole app shares one button implementation and its classes.
//! Keeping a local `ButtonVariant` enum lets existing call sites stay unchanged
//! while the underlying markup/classes come from `dioxus_components`.

use dioxus::prelude::*;
use dioxus_components::{
    Badge as DcBadge, BadgeVariant as DcBadgeVariant, Button as DcButton, ButtonSize,
    ButtonVariant as DcButtonVariant,
};

#[derive(Clone, PartialEq, Default)]
pub enum ButtonVariant {
    #[default]
    Primary,
    Ghost,
    Subtle,
    Danger,
}

impl ButtonVariant {
    /// Map our semantic variant onto `dioxus_components`' palette. The tokens
    /// (`--color-primary`, `--color-secondary`, `--color-destructive`,
    /// `--color-accent`) are defined in `assets/tailwind-input.css` to match the
    /// app's dark theme.
    fn dc_variant(&self) -> DcButtonVariant {
        match self {
            // Brand purple fill (--color-primary).
            ButtonVariant::Primary => DcButtonVariant::Default,
            // Transparent; hover picks up --color-accent (brand purple).
            ButtonVariant::Ghost => DcButtonVariant::Ghost,
            // Subtle surface fill (--color-secondary).
            ButtonVariant::Subtle => DcButtonVariant::Secondary,
            // Red fill (--color-destructive).
            ButtonVariant::Danger => DcButtonVariant::Destructive,
        }
    }
}

/// Badge variants, mapped onto `dioxus_components`' palette.
#[derive(Clone, PartialEq, Default)]
pub enum BadgeVariant {
    #[default]
    Default,
    Secondary,
    Destructive,
    Outline,
}

impl BadgeVariant {
    fn dc_variant(&self) -> DcBadgeVariant {
        match self {
            BadgeVariant::Default => DcBadgeVariant::Default,
            BadgeVariant::Secondary => DcBadgeVariant::Secondary,
            BadgeVariant::Destructive => DcBadgeVariant::Destructive,
            BadgeVariant::Outline => DcBadgeVariant::Outline,
        }
    }
}

#[component]
pub fn Badge(children: Element, variant: Option<BadgeVariant>, class: Option<String>) -> Element {
    let v = variant.unwrap_or_default();
    let extra = class.unwrap_or_default();
    rsx! {
        DcBadge {
            variant: v.dc_variant(),
            class: "{extra}",
            {children}
        }
    }
}

#[component]
pub fn Button(
    children: Element,
    onclick: Option<EventHandler<MouseEvent>>,
    variant: Option<ButtonVariant>,
    title: Option<String>,
    class: Option<String>,
    disabled: Option<bool>,
) -> Element {
    let v = variant.unwrap_or_default();
    let extra = class.unwrap_or_default();
    let title_attr = title.unwrap_or_default();
    // `dioxus_components::Button` has no `title` prop, so the native tooltip is
    // surfaced on a wrapping span (an empty title is a harmless no-op).
    rsx! {
        span { title: "{title_attr}",
            DcButton {
                variant: v.dc_variant(),
                size: ButtonSize::Sm,
                disabled: disabled,
                onclick: move |evt| {
                    if let Some(cb) = onclick {
                        cb.call(evt);
                    }
                },
                class: "{extra}",
                {children}
            }
        }
    }
}

#[component]
pub fn IconButton(
    children: Element,
    onclick: Option<EventHandler<MouseEvent>>,
    title: String,
    variant: Option<ButtonVariant>,
    class: Option<String>,
    disabled: Option<bool>,
) -> Element {
    let v = variant.unwrap_or(ButtonVariant::Ghost);
    let extra = class.unwrap_or_default();
    rsx! {
        span { title: "{title}",
            DcButton {
                variant: v.dc_variant(),
                size: ButtonSize::IconSm,
                disabled: disabled,
                onclick: move |evt| {
                    if let Some(cb) = onclick {
                        cb.call(evt);
                    }
                },
                class: "{extra}",
                {children}
            }
        }
    }
}
