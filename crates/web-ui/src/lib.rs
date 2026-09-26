//! omenic-web-ui：Web 前端（dioxus LiveView），按 Dioxus 社区惯例分层。
//!
//! - [`components`]：可复用 UI 组件（chat / sidebar / ui atoms / icons / taskpanel）
//! - [`views`]：页面级组合（workspace / config / stats）
//! - [`layouts`]：共享布局壳（AppFrame 三列框架）
//! - [`utils`]：纯函数工具（markdown 渲染）
//!
//! 数据来源与转换层不在本 crate：`web-client`（daemon 客户端）与
//! `web-state`（DTO + wire→UI 投影）由 TUI 共享，故独立成 crate。

pub mod components;
pub mod layouts;
pub mod utils;
pub mod views;

pub use views::workspace::Workspace;
