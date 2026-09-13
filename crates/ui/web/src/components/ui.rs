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

/// 通用下拉菜单：触发按钮 + 向上弹出的选项列表，可复用。
/// 点击组件外任意区域自动收起：组件自渲染一层透明遮罩（z-40），
/// 触发按钮与菜单浮在其上（z-50），因此点菜单内正常选择、点外部命中遮罩收起。
#[component]
pub fn Dropdown(
    /// 触发按钮上的文字（含 ▾ 箭头由调用方拼好）
    label: String,
    /// 菜单顶部的小标题
    header: String,
    /// 选项列表：(显示文字, 值)
    items: Vec<(String, String)>,
    /// 当前选中值（命中则高亮）
    active_value: String,
    /// 触发按钮/选项是否用等宽字体
    #[props(default = false)]
    mono: bool,
    /// 菜单宽度类（默认 min-w-[180px]）
    #[props(optional)]
    menu_class: Option<String>,
    /// 选中某项
    on_select: EventHandler<String>,
) -> Element {
    let mut open = use_signal(|| false);
    let menu_class = menu_class.unwrap_or_else(|| "min-w-[180px]".to_string());

    let base = "text-[11px] px-2 py-1 h-7 flex items-center justify-center gap-1 rounded text-muted-foreground border border-subtle bg-transparent hover:border-hover transition-colors";
    let trigger_class = if mono {
        format!("{base} font-mono")
    } else {
        base.to_string()
    };
    let item_text_class = if mono { "font-mono" } else { "" };

    rsx! {
        div { class: "relative",
            button {
                class: "{trigger_class} relative z-50",
                r#type: "button",
                onclick: move |_| open.set(!open()),
                "{label}"
            }
            if open() {
                // 透明遮罩：吃掉外部点击
                div {
                    class: "fixed inset-0 z-40",
                    onclick: move |_| open.set(false),
                }
                div { class: "absolute bottom-full left-0 mb-1 z-50 bg-surface-elevated border border-subtle rounded-lg shadow-lg py-1 {menu_class}",
                    div { class: "px-3 py-1.5 text-[10px] font-semibold text-muted uppercase tracking-wide", "{header}" }
                    for (item_label, item_value) in items.iter() {
                        {
                            let item_label = item_label.clone();
                            let item_value = item_value.clone();
                            let is_active = item_value == active_value;
                            rsx! {
                                div {
                                    key: "{item_value}",
                                    class: if is_active { "px-3 py-1.5 text-xs cursor-pointer bg-hover text-foreground font-medium" } else { "px-3 py-1.5 text-xs cursor-pointer text-muted-foreground hover:bg-hover hover:text-foreground" },
                                    onclick: move |_| {
                                        on_select.call(item_value.clone());
                                        open.set(false);
                                    },
                                    span { class: "{item_text_class}", "{item_label}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
