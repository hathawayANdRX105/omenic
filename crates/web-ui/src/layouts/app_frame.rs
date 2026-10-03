//! 共享布局壳：dsh AppFrame 两列框架（双栏侧栏 270 / 折叠 80 / 中栏）。
//!
//! 只管几何与层级，不碰数据——侧栏、中栏头、正文都由调用方以 Element 传入。
//! 宽度固定为 ui-kit DualPaneNav 双栏几何（kit 自管 270/80 过渡），
//! 拖拽调宽与预设逻辑已随单栏侧栏退役。

use dioxus::prelude::*;

/// 侧栏 + 中栏两列的 grid 模板。折叠态 80px（rail），展开态 270px；
/// 中栏恒为 `minmax(0,1fr)`，可收缩到 0（无独立顶栏行）。
pub fn grid_cols(collapsed: bool) -> String {
    if collapsed {
        "80px minmax(0,1fr)".to_string()
    } else {
        "270px minmax(0,1fr)".to_string()
    }
}

/// 两列框架根容器。
///
/// - `sidebar`：侧栏 Element（ui-kit DualPaneNav 双栏侧栏）
/// - `header`：中栏顶部头（面包屑 / 页标题）Element
/// - `children`：中栏正文 Element
#[component]
pub fn AppFrame(collapsed: bool, sidebar: Element, header: Element, children: Element) -> Element {
    rsx! {
        div { class: "grid h-screen w-screen bg-background overflow-hidden relative select-none grid-rows-[minmax(0,1fr)]",
            style: "grid-template-columns: {grid_cols(collapsed)};",
            {sidebar}
            div { class: "min-w-0 flex flex-col bg-background overflow-hidden",
                {header}
                {children}
            }
        }
    }
}
