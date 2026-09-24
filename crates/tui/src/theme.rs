//! theme.rs — D11：enhanced 外壳的颜色/样式唯一入口（token 名对齐 web
//! 的 brand / dim / danger）。全部走 ratatui 命名色：不写 hex 色值、不写
//! ESC 转义字面量（`tests/theme_lint.rs` 扫全 `crates/tui/src` 钉住），
//! 配色决定权留给终端调色板，浅色/深色终端都可读（decisions D11）。
//!
//! `crates/tui/src` 内任何样式都必须从本模块出发；ui/* 只许 import 本模块
//! 的语义函数，不许把 `Color` 构造泄漏到调用点。

use ratatui::style::{Color, Modifier, Style};

/// 正文默认：不着色，跟随终端前景色。
pub fn base() -> Style {
    Style::default()
}

/// brand（web `--color-brand` 同族）：强调、用户侧、运行中。
pub fn brand() -> Style {
    Style::default().fg(Color::LightBlue)
}

/// brand 加粗：状态点、composer 提示符、transcript 用户前缀。
pub fn brand_bold() -> Style {
    brand().add_modifier(Modifier::BOLD)
}

/// dim（web `--color-dim` 同族）：辅助信息、按键提示、空闲态、工具行。
pub fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

/// danger（web `--color-danger` 同族）：错误、中断、退出确认。
pub fn danger() -> Style {
    Style::default().fg(Color::LightRed)
}
