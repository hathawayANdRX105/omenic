//! 自绘 SVG 图标集（对齐 dsh `ic_ds_*` 约定：`fill=none`+`stroke=currentColor`，
//! 颜色随 CSS `color`，默认 16px 槽）。
//!
//! 注意：组件名必须大写开头，rsx! 里小写会被当作 HTML 标签。

use dioxus::prelude::*;

/// 所有图标的公共 svg 外壳。
#[component]
fn Base(children: Element, size: u32, class: String) -> Element {
    rsx! {
        svg {
            class: "{class}",
            xmlns: "http://www.w3.org/2000/svg",
            width: "{size}",
            height: "{size}",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            {children}
        }
    }
}

#[component]
pub fn PanelLeft(size: Option<u32>, class: Option<String>) -> Element {
    let size = size.unwrap_or(16);
    let class = class.unwrap_or_default();
    rsx! {
        Base { class, size,
            rect { x: "3", y: "3", width: "18", height: "18", rx: "2" }
            path { d: "M9 3v18" }
        }
    }
}

#[component]
pub fn Plus(size: Option<u32>, class: Option<String>) -> Element {
    let size = size.unwrap_or(16);
    let class = class.unwrap_or_default();
    rsx! {
        Base { class, size,
            path { d: "M12 5v14" }
            path { d: "M5 12h14" }
        }
    }
}

#[component]
pub fn Trash(size: Option<u32>, class: Option<String>) -> Element {
    let size = size.unwrap_or(16);
    let class = class.unwrap_or_default();
    rsx! {
        Base { class, size,
            path { d: "M3 6h18" }
            path { d: "M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" }
            path { d: "M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6" }
            path { d: "M10 11v6" }
            path { d: "M14 11v6" }
        }
    }
}

#[component]
pub fn Search(size: Option<u32>, class: Option<String>) -> Element {
    let size = size.unwrap_or(16);
    let class = class.unwrap_or_default();
    rsx! {
        Base { class, size,
            circle { cx: "11", cy: "11", r: "8" }
            path { d: "m21 21-4.3-4.3" }
        }
    }
}

#[component]
pub fn ChevronDown(size: Option<u32>, class: Option<String>) -> Element {
    let size = size.unwrap_or(12);
    let class = class.unwrap_or_default();
    rsx! {
        Base { class, size,
            path { d: "m6 9 6 6 6-6" }
        }
    }
}

#[component]
pub fn ChevronRight(size: Option<u32>, class: Option<String>) -> Element {
    let size = size.unwrap_or(12);
    let class = class.unwrap_or_default();
    rsx! {
        Base { class, size,
            path { d: "m9 6 6 6-6 6" }
        }
    }
}

#[component]
pub fn ArrowUp(size: Option<u32>, class: Option<String>) -> Element {
    let size = size.unwrap_or(16);
    let class = class.unwrap_or_default();
    rsx! {
        Base { class, size,
            path { d: "M12 19V5" }
            path { d: "m5 12 7-7 7 7" }
        }
    }
}

#[component]
pub fn X(size: Option<u32>, class: Option<String>) -> Element {
    let size = size.unwrap_or(16);
    let class = class.unwrap_or_default();
    rsx! {
        Base { class, size,
            path { d: "M18 6 6 18" }
            path { d: "m6 6 12 12" }
        }
    }
}

#[component]
pub fn Folder(size: Option<u32>, class: Option<String>) -> Element {
    let size = size.unwrap_or(16);
    let class = class.unwrap_or_default();
    rsx! {
        Base { class, size,
            path { d: "M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.909l-.06-.062a1 1 0 0 0-1.35-.927l-.4.231a2 2 0 0 1-1.69.908H4a2 2 0 0 0-2 2v10a2 2 0 0 0 2 2Z" }
        }
    }
}

#[component]
pub fn Gear(size: Option<u32>, class: Option<String>) -> Element {
    let size = size.unwrap_or(16);
    let class = class.unwrap_or_default();
    rsx! {
        Base { class, size,
            path { d: "M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" }
            circle { cx: "12", cy: "12", r: "3" }
        }
    }
}

#[component]
pub fn Chart(size: Option<u32>, class: Option<String>) -> Element {
    let size = size.unwrap_or(16);
    let class = class.unwrap_or_default();
    rsx! {
        Base { class, size,
            path { d: "M3 3v16a2 2 0 0 0 2 2h16" }
            path { d: "M7 16v-5" }
            path { d: "M12 16V8" }
            path { d: "M17 16v-8" }
        }
    }
}

#[component]
pub fn Copy(size: Option<u32>, class: Option<String>) -> Element {
    let size = size.unwrap_or(16);
    let class = class.unwrap_or_default();
    rsx! {
        Base { class, size,
            rect { x: "9", y: "9", width: "12", height: "12", rx: "2" }
            path { d: "M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" }
        }
    }
}

#[component]
pub fn Check(size: Option<u32>, class: Option<String>) -> Element {
    let size = size.unwrap_or(16);
    let class = class.unwrap_or_default();
    rsx! {
        Base { class, size,
            path { d: "M20 6 9 17l-5-5" }
        }
    }
}

#[component]
pub fn Terminal(size: Option<u32>, class: Option<String>) -> Element {
    let size = size.unwrap_or(16);
    let class = class.unwrap_or_default();
    rsx! {
        Base { class, size,
            path { d: "m4 17 6-6-6-6" }
            path { d: "M12 19h8" }
        }
    }
}

#[component]
pub fn SquareCheck(size: Option<u32>, class: Option<String>) -> Element {
    let size = size.unwrap_or(16);
    let class = class.unwrap_or_default();
    rsx! {
        Base { class, size,
            rect { x: "3", y: "3", width: "18", height: "18", rx: "4" }
            path { d: "m8.5 12 2.5 2.5 5-5" }
        }
    }
}

#[component]
pub fn Paperclip(size: Option<u32>, class: Option<String>) -> Element {
    let size = size.unwrap_or(16);
    let class = class.unwrap_or_default();
    rsx! {
        Base { class, size,
            path { d: "m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48" }
        }
    }
}

/// 品牌字标（Logo 行）：omenic + 品牌蓝圆点。
#[component]
pub fn Wordmark() -> Element {
    rsx! {
        span { class: "flex items-baseline gap-1.5 select-none",
            span { class: "text-[18px] leading-6 font-semibold tracking-[0.04em] text-label", "omenic" }
            span { class: "w-1.5 h-1.5 rounded-full bg-brand translate-y-[-2px]" }
        }
    }
}
