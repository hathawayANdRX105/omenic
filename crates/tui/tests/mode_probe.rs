//! mode_probe — route §3 裁决契约：auto / linear / enhanced 的门判定。
//!
//! 不起 daemon、不进事件循环：构造 [`TermProbe`] 快照打纯谓词。T1 下门全
//! 过也落 linear（enhanced 分支 T2 启用），所以断言分两层——门
//! （`enhanced_eligible`）能红，裁决（`resolve_mode`）恒 linear 不会因
//! T2 启用 enhanced 而误红。

use omenic_tui::{MuxKind, TermProbe, TuiMode, enhanced_eligible, resolve_mode};

/// 门全过的基线探针：双 TTY、有色、合法 TERM、80×24、裸终端。
fn good_probe() -> TermProbe {
    TermProbe {
        stdin_tty: true,
        stdout_tty: true,
        color: true,
        term: Some("xterm-256color".to_string()),
        size: (80, 24),
        inside_mux: None,
    }
}

/// bug：auto 在复用器里 / 小终端误开 enhanced → 全屏渲染写进不支持的
/// 环境，画面写崩（route §4 测试表第一条）。
#[test]
fn auto_mode_avoids_mux_and_small_terminal() {
    // 复用器里尺寸再大也不许开 enhanced（tmux / screen / zellij 三家）。
    for mux in [MuxKind::Tmux, MuxKind::Screen, MuxKind::Zellij] {
        let probe = TermProbe {
            inside_mux: Some(mux),
            ..good_probe()
        };
        assert!(
            !enhanced_eligible(&probe),
            "mux {mux:?} must not be eligible: {probe:?}"
        );
        assert_eq!(resolve_mode(TuiMode::Auto, &probe), TuiMode::Linear);
    }
    // 尺寸低于 44×12 下限（含未报尺寸的 (0, 0)）。
    for size in [(43, 12), (44, 11), (0, 0)] {
        let probe = TermProbe {
            size,
            ..good_probe()
        };
        assert!(
            !enhanced_eligible(&probe),
            "size {size:?} must not be eligible: {probe:?}"
        );
        assert_eq!(resolve_mode(TuiMode::Auto, &probe), TuiMode::Linear);
    }
    // 基线：门全过时 T1 仍落 linear（enhanced 分支 T2 启用）。
    assert_eq!(resolve_mode(TuiMode::Auto, &good_probe()), TuiMode::Linear);
}

/// bug：`NO_COLOR` / `--no-color` / `TERM=dumb` / 非 TTY 被忽略 →
/// 有颜色的输出污染管道（route §4 测试表第二条）。
#[test]
fn dumb_or_no_color_forces_linear() {
    // `--tui linear` 请求：门再齐也强制 linear。
    assert_eq!(
        resolve_mode(TuiMode::Linear, &good_probe()),
        TuiMode::Linear
    );
    // NO_COLOR / --no-color：探针 color=false → 门不过 → linear。
    let no_color = TermProbe {
        color: false,
        ..good_probe()
    };
    assert!(!enhanced_eligible(&no_color));
    assert_eq!(resolve_mode(TuiMode::Auto, &no_color), TuiMode::Linear);
    // TERM=dumb、TERM 未设置（None）、TERM 空串都按 dumb 处理。
    for term in [Some("dumb".to_string()), None, Some(String::new())] {
        let probe = TermProbe {
            term,
            ..good_probe()
        };
        assert!(
            !enhanced_eligible(&probe),
            "term must force linear: {probe:?}"
        );
        assert_eq!(resolve_mode(TuiMode::Auto, &probe), TuiMode::Linear);
    }
    // 管道/重定向：stdin / stdout 任一非 TTY 即 linear。
    for (stdin_tty, stdout_tty) in [(false, true), (true, false)] {
        let probe = TermProbe {
            stdin_tty,
            stdout_tty,
            ..good_probe()
        };
        assert!(!enhanced_eligible(&probe));
        assert_eq!(resolve_mode(TuiMode::Auto, &probe), TuiMode::Linear);
    }
    // `--tui enhanced` 在 T1 也落 linear（route §3：T1 的 enhanced 就是 linear）。
    assert_eq!(
        resolve_mode(TuiMode::Enhanced, &good_probe()),
        TuiMode::Linear
    );
}
