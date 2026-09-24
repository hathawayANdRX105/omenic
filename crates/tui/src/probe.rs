//! probe.rs — 终端能力采集（route §3 `TermProbe` 结构，字段名是契约）。
//!
//! 只读环境变量与 fd，不改任何终端状态（T1 全程无 raw mode / 无
//! alternate screen，route §8）。复用器检测显式覆盖常见三家：环境变量
//! `TMUX` / `ZELLIJ` / `STY`，或 `TERM` 形态（`tmux-` / `screen-` 前缀、
//! 含 `zellij`）。

use std::io::IsTerminal;

/// 终端能力快照（route §3 契约字段，不许改名）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermProbe {
    /// stdin 是否 TTY（管道喂入 → false → linear）。
    pub stdin_tty: bool,
    /// stdout 是否 TTY（重定向到文件/管道 → false → linear）。
    pub stdout_tty: bool,
    /// 是否允许颜色（`NO_COLOR` 非空 → false；`--no-color` 在 `run()` 里覆写）。
    pub color: bool,
    /// `TERM` 原值（未设置为 `None`，按 dumb 处理）。
    pub term: Option<String>,
    /// 终端尺寸（列, 行）；取不到时 (0, 0)。
    pub size: (u16, u16),
    /// 检出的终端复用器（`None` = 裸终端）。
    pub inside_mux: Option<MuxKind>,
}

/// 终端复用器种类（route §3：tmux / Screen / Zellij）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MuxKind {
    /// tmux：`TMUX` 存在，或 `TERM` 以 `tmux-` 开头。
    Tmux,
    /// GNU screen：`STY` 存在，或 `TERM` 以 `screen-` 开头。
    Screen,
    /// zellij：`ZELLIJ` 存在，或 `TERM` 含 `zellij`。
    Zellij,
}

impl TermProbe {
    /// 从环境 + fd 采集（route §3 签名，不许改）。
    pub fn from_env() -> Self {
        let term = std::env::var("TERM").ok();
        let size = crossterm::terminal::size().unwrap_or((0, 0));
        TermProbe {
            stdin_tty: std::io::stdin().is_terminal(),
            stdout_tty: std::io::stdout().is_terminal(),
            color: color_from_env(),
            term,
            size,
            inside_mux: detect_mux(),
        }
    }
}

/// 颜色开关：`NO_COLOR` 非空 → 不允许（与 core/cli 同一读法）。
fn color_from_env() -> bool {
    !matches!(std::env::var("NO_COLOR"), Ok(v) if !v.is_empty())
}

/// 复用器检测：显式环境变量优先，回落 `TERM` 形态。
fn detect_mux() -> Option<MuxKind> {
    if std::env::var_os("TMUX").is_some() {
        return Some(MuxKind::Tmux);
    }
    if std::env::var_os("ZELLIJ").is_some() {
        return Some(MuxKind::Zellij);
    }
    if std::env::var_os("STY").is_some() {
        return Some(MuxKind::Screen);
    }
    let term = std::env::var("TERM").ok()?;
    if term.starts_with("tmux-") {
        Some(MuxKind::Tmux)
    } else if term.starts_with("screen-") {
        Some(MuxKind::Screen)
    } else if term.contains("zellij") {
        Some(MuxKind::Zellij)
    } else {
        None
    }
}
