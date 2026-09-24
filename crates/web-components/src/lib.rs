//! omenic-web-components：dsh 风格自研组件（零组件库依赖）。
//!
//! - [`icons`]：自绘 SVG 图标集
//! - [`ui`]：atoms（Button/IconButton/Badge/Dropdown/Modal/Spinner）
//! - [`sidebar`]：三列框架的侧栏（dsh 280px / rail 56px）
//! - [`chat`]：会话区（748px 消息列 + 浮动 composer）
//! - [`taskpanel`]：composer 上方的任务 dock 卡片（含 FilterChip）

pub mod chat;
pub mod icons;
pub mod sidebar;
pub mod taskpanel;
pub mod ui;
