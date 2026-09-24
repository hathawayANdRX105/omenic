//! mode.rs — 探针 → linear/enhanced 裁决（route §3 签名，不许改）。
//!
//! 判定顺序即 route §3：`--tui linear` 早退 → [`enhanced_eligible`] 门
//! （双 TTY + 颜色 + TERM 合法 + ≥44×12 + 无复用器）→ 门过返回
//! [`TuiMode::Enhanced`]。门不齐一定落 linear：终端不对时永不进全屏路径。
//! T1 的「门过也恒 linear」临时分支已随 T2 启用删除（route §7 过渡层清零，
//! 不留兼容 shim）。

use crate::probe::TermProbe;

/// TUI 渲染档位（route §3 契约：Auto / Enhanced / Linear）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum TuiMode {
    /// 探针裁决：门全过走 enhanced，否则 linear。
    Auto,
    /// 强制 enhanced 全屏渲染（仍受门约束：门不齐落 linear）。
    Enhanced,
    /// 强制裸文本行渲染（route §3 的 T1 基线）。
    Linear,
}

/// enhanced 是否可行：双 TTY + 有颜色 + TERM 合法 + ≥44×12 + 无复用器。
///
/// 单拆出来导出，是为了让「mux/尺寸判定写错」这类 bug 在 T1 就能红：
/// auto 档 T1 恒落 linear，不测门则 route §4 测试表的第一条永远绿。
pub fn enhanced_eligible(probe: &TermProbe) -> bool {
    probe.stdin_tty
        && probe.stdout_tty
        && probe.color
        && term_ok(probe)
        && size_ok(probe)
        && probe.inside_mux.is_none()
}

/// TERM 合法性：未设置 / 空串按 dumb 处理（route §3 边界），dumb 不上 TUI。
fn term_ok(probe: &TermProbe) -> bool {
    matches!(probe.term.as_deref(), Some(t) if !t.is_empty() && t != "dumb")
}

/// 尺寸下限（列, 行）。`terminal::size` 取不到时是 (0, 0)，自然不过下限。
const MIN_COLS: u16 = 44;
const MIN_ROWS: u16 = 12;

fn size_ok(probe: &TermProbe) -> bool {
    probe.size.0 >= MIN_COLS && probe.size.1 >= MIN_ROWS
}

/// 裁决渲染档位（route §3 签名，不许改）。
///
/// `Linear` 请求早退；门不过 → linear；门过 → enhanced（T2 起启用：
/// T1 临时的「门过也恒 linear」分支已删，`--tui enhanced` 不再被降级）。
pub fn resolve_mode(requested: TuiMode, probe: &TermProbe) -> TuiMode {
    if matches!(requested, TuiMode::Linear) {
        return TuiMode::Linear;
    }
    if !enhanced_eligible(probe) {
        return TuiMode::Linear;
    }
    TuiMode::Enhanced
}
