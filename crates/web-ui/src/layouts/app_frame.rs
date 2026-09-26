//! 共享布局壳：dsh AppFrame 三列框架（侧栏 280 / 折叠 rail 56 / 中栏）。
//!
//! 只管几何与层级，不碰数据——侧栏、中栏头、正文都由调用方以 Element 传入。

use dioxus::prelude::*;

/// 侧栏 + 中栏两列的 grid 模板。折叠态是 56px rail，展开态是调用方给的宽度；
/// 中栏恒为 `minmax(0,1fr)`，可收缩到 0（无独立顶栏行）。
pub fn grid_cols(collapsed: bool, width: usize) -> String {
    if collapsed {
        "56px minmax(0,1fr)".to_string()
    } else {
        format!("{width}px minmax(0,1fr)")
    }
}

/// 三列框架根容器：拖拽夹取侧栏宽度靠根节点的 mousemove/mouseup 冒泡。
///
/// - `sidebar`：侧栏 Element
/// - `header`：中栏顶部头（面包屑 / 统计页标题）Element
/// - `children`：中栏正文 Element
#[component]
pub fn AppFrame(
    collapsed: bool,
    width: usize,
    on_resize: Callback<MouseEvent>,
    on_resize_end: Callback<MouseEvent>,
    sidebar: Element,
    header: Element,
    children: Element,
) -> Element {
    rsx! {
        div { class: "grid h-screen w-screen bg-base overflow-hidden relative select-none",
            style: "grid-template-columns: {grid_cols(collapsed, width)};",
            onmousemove: on_resize,
            onmouseup: on_resize_end,
            {sidebar}
            div { class: "min-w-0 flex flex-col bg-base overflow-hidden",
                {header}
                {children}
            }
        }
    }
}
