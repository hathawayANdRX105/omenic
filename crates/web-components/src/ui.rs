//! 自研 UI atoms（对齐 dsh ui-primitives 规格：Button h36 r18、Menu r12、
//! Spinner 10px 圆环、Modal r24 + blur 遮罩）。不依赖任何组件库。

use dioxus::prelude::*;

use crate::icons::{Check, ChevronDown, X};

// ── Button ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Default)]
pub enum ButtonVariant {
    /// 近白胶囊 + 墨色文字（dsh dark 的 button-primary-fill）
    #[default]
    Primary,
    /// 透明底，hover 白 8%
    Ghost,
    /// 1px 边框透明底（dsh outline）
    Outline,
    /// 品牌蓝实底 + 白字（dsh button-info-fill）
    Info,
    /// 错误红
    Danger,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum ButtonSize {
    /// h36 r18 14/22（dsh 基准胶囊）
    #[default]
    Md,
    /// h28 r14 12/18（dsh .sm）
    Sm,
}

#[component]
pub fn Button(
    children: Element,
    onclick: Option<EventHandler<MouseEvent>>,
    variant: Option<ButtonVariant>,
    size: Option<ButtonSize>,
    title: Option<String>,
    class: Option<String>,
    disabled: Option<bool>,
) -> Element {
    let variant = variant.unwrap_or_default();
    let size = size.unwrap_or_default();
    let extra = class.unwrap_or_default();
    let title_attr = title.unwrap_or_default();

    let variant_class = match variant {
        ButtonVariant::Primary => "bg-brand-fill text-ink hover:bg-[#ebeef2]",
        ButtonVariant::Ghost => {
            "bg-transparent text-label-2 hover:bg-ihover hover:text-label active:bg-iactive"
        }
        ButtonVariant::Outline => {
            "border border-b2 bg-transparent text-label-2 hover:bg-ihover hover:text-label"
        }
        ButtonVariant::Info => "bg-brand text-white hover:bg-brand-hover",
        ButtonVariant::Danger => "bg-danger text-white hover:bg-danger-2",
    };
    let size_class = match size {
        ButtonSize::Md => "h-9 px-3.5 rounded-[18px] text-[14px] leading-[22px]",
        ButtonSize::Sm => "h-7 px-2.5 rounded-[14px] text-[12px] leading-[18px]",
    };

    rsx! {
        button {
            r#type: "button",
            title: "{title_attr}",
            disabled: disabled.unwrap_or(false),
            class: "inline-flex items-center justify-center gap-1.5 font-medium select-none cursor-pointer transition-colors disabled:opacity-40 disabled:cursor-not-allowed {variant_class} {size_class} {extra}",
            onclick: move |evt| {
                if let Some(cb) = onclick {
                    cb.call(evt);
                }
            },
            {children}
        }
    }
}

/// 28×28 圆形图标钮（dsh IconActions / 侧栏动作钮规格）。
#[component]
pub fn IconButton(
    children: Element,
    onclick: Option<EventHandler<MouseEvent>>,
    title: Option<String>,
    class: Option<String>,
    disabled: Option<bool>,
) -> Element {
    let extra = class.unwrap_or_default();
    let title_attr = title.unwrap_or_default();
    rsx! {
        button {
            r#type: "button",
            title: "{title_attr}",
            disabled: disabled.unwrap_or(false),
            class: "inline-flex items-center justify-center w-7 h-7 rounded-full text-label-3 hover:bg-ihover hover:text-label-2 active:bg-iactive transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed {extra}",
            onclick: move |evt| {
                if let Some(cb) = onclick {
                    cb.call(evt);
                }
            },
            {children}
        }
    }
}

// ── Badge（状态胶囊）────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Default)]
pub enum BadgeVariant {
    #[default]
    Neutral,
    Brand,
    Success,
    Warn,
    Danger,
}

#[component]
pub fn Badge(children: Element, variant: Option<BadgeVariant>, class: Option<String>) -> Element {
    let variant = variant.unwrap_or_default();
    let extra = class.unwrap_or_default();
    let variant_class = match variant {
        BadgeVariant::Neutral => "bg-layer-2 text-label-3 border-b2",
        BadgeVariant::Brand => "bg-chip-brand text-brand-300 border-transparent",
        BadgeVariant::Success => "bg-chip-success text-success-2 border-transparent",
        BadgeVariant::Warn => "bg-chip-warn text-warn-2 border-transparent",
        BadgeVariant::Danger => "bg-chip-danger text-danger border-transparent",
    };
    rsx! {
        span { class: "inline-flex items-center rounded-md border px-1.5 py-0.5 text-[11px] leading-4 font-medium {variant_class} {extra}",
            {children}
        }
    }
}

// ── Spinner（dsh：10px 圆环 + business 蓝顶弧 700ms）────────────────────────

#[component]
pub fn Spinner(class: Option<String>) -> Element {
    let extra = class.unwrap_or_default();
    rsx! { span { class: "spinner-ring {extra}" } }
}

// ── Dropdown（dsh Menu 弹层）────────────────────────────────────────────────

/// 通用下拉菜单：触发钮 + 弹出选项列表。
/// 点击组件外任意区域自动收起：组件自渲染一层透明遮罩（z-40），
/// 触发按钮与菜单浮在其上（z-50）。
#[component]
pub fn Dropdown(
    /// 触发按钮上的文字（含 ▾ 由调用方拼好或走 label + 自带 chevron）
    label: String,
    /// 菜单顶部的小标题
    header: String,
    /// 选项列表：(显示文字, 值)
    items: Vec<(String, String)>,
    /// 当前选中值（命中则高亮 + check）
    active_value: String,
    /// 触发按钮/选项是否用等宽字体
    #[props(default = false)]
    mono: bool,
    /// 菜单宽度类（默认 min-w-[180px]）
    #[props(optional)]
    menu_class: Option<String>,
    /// 菜单弹在触发件下方（默认向上弹，供 composer 使用）
    #[props(default = false)]
    open_down: bool,
    on_select: EventHandler<String>,
) -> Element {
    let mut open = use_signal(|| false);
    let menu_class = menu_class.unwrap_or_else(|| "min-w-[190px]".to_string());

    let trigger_base = "inline-flex items-center gap-1 h-7 px-2 rounded-[10px] text-[13px] leading-5 text-label-2 hover:bg-ihover hover:text-label active:bg-iactive transition-colors cursor-pointer select-none";
    let trigger_class = if mono {
        format!("{trigger_base} font-mono")
    } else {
        trigger_base.to_string()
    };
    let item_text_class = if mono { "font-mono" } else { "" };
    let pos_class = if open_down {
        "top-full left-0 mt-1"
    } else {
        "bottom-full left-0 mb-1"
    };

    rsx! {
        div { class: "relative",
            button {
                r#type: "button",
                class: "{trigger_class} relative z-50",
                onclick: move |_| open.set(!open()),
                span { "{label}" }
                span { class: "text-label-3",
                    ChevronDown { size: 12 }
                }
            }
            if open() {
                // 透明遮罩：吃掉外部点击
                div {
                    class: "fixed inset-0 z-40",
                    onclick: move |_| open.set(false),
                }
                div { class: "absolute {pos_class} z-50 rounded-xl border border-binv bg-menu shadow-lv3 p-1 {menu_class}",
                    div { class: "px-2.5 py-1.5 text-[12px] leading-4 text-caption select-none", "{header}" }
                    for (item_label, item_value) in items.iter() {
                        {
                            let item_label = item_label.clone();
                            let item_value = item_value.clone();
                            let is_active = item_value == active_value;
                            let item_class = if is_active {
                                "min-h-[34px] px-2.5 py-1.5 rounded-[10px] flex items-center justify-between gap-2 text-[14px] leading-[22px] cursor-pointer bg-ihover text-label"
                            } else {
                                "min-h-[34px] px-2.5 py-1.5 rounded-[10px] flex items-center justify-between gap-2 text-[14px] leading-[22px] cursor-pointer text-label-2 hover:bg-ihover hover:text-label"
                            };
                            rsx! {
                                div {
                                    key: "{item_value}",
                                    class: "{item_class}",
                                    onclick: move |_| {
                                        on_select.call(item_value.clone());
                                        open.set(false);
                                    },
                                    span { class: "truncate {item_text_class}", "{item_label}" }
                                    if is_active {
                                        Check { size: 14, class: "shrink-0 text-label-2" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ── Modal（dsh：mask 24%→dark 50% + blur 2px，卡片 r24 layer-2 lv3）────────

#[component]
pub fn Modal(
    children: Element,
    on_close: EventHandler<()>,
    /// 卡片宽度类（默认 w-[380px]）
    #[props(optional)]
    width_class: Option<String>,
    /// 垂直位置：居中（默认）或顶部 pt-[14vh]
    #[props(default = false)]
    top_aligned: bool,
) -> Element {
    let width = width_class.unwrap_or_else(|| "w-[380px]".to_string());
    let align = if top_aligned {
        "items-start pt-[14vh]"
    } else {
        "items-center"
    };
    rsx! {
        div {
            class: "fixed inset-0 z-[1000] p-6 bg-mask-1 backdrop-blur-[2px] flex justify-center {align}",
            onclick: move |_| on_close.call(()),
            div {
                class: "{width} max-w-full rounded-3xl border border-binv bg-layer-2 shadow-lv3 overflow-hidden",
                onclick: move |e: MouseEvent| e.stop_propagation(),
                {children}
            }
        }
    }
}

/// Modal 标题行（右上 28×28 关闭钮）。
#[component]
pub fn ModalHeader(title: String, on_close: EventHandler<()>) -> Element {
    rsx! {
        div { class: "flex items-center justify-between px-6 pt-5 pb-3",
            span { class: "text-[16px] leading-6 font-medium text-label", "{title}" }
            IconButton { title: "关闭", onclick: move |_| on_close.call(()), X { size: 16 } }
        }
    }
}
