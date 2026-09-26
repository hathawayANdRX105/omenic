//! mode_probe — route §3 裁决契约：auto / linear / enhanced / inline 的门判定。
//!
//! 不起 daemon、不进事件循环：构造 [`TermProbe`] 快照打纯谓词。断言分两
//! 层——门（[`enhanced_eligible`]）与裁决（[`resolve_mode`]）：门全过才落
//! enhanced；门不齐（mux / 小终端 / dumb / 无色 / 非 TTY）一律 linear。
//! T8 增量：显式 `--tui inline` 走同一扇门、auto 永不选 inline、CLI 第四臂
//! 解析（`value_enum` 四臂 + 非法值报错）。

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
    // 基线：门全过时 auto 裁决 enhanced（T2 起启用）。
    assert_eq!(
        resolve_mode(TuiMode::Auto, &good_probe()),
        TuiMode::Enhanced
    );
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
    // `--tui enhanced` 显式请求 + 门全过 → enhanced（门不齐仍 linear，见上）。
    assert_eq!(
        resolve_mode(TuiMode::Enhanced, &good_probe()),
        TuiMode::Enhanced
    );
}

/// T8 第四档门的全部反例（mux / 小终端 / 无色 / dumb / 非 TTY）：
/// 探针门测试增量的公共数据源（route §3 T8 设计注记 ①）。
fn ineligible_probes() -> Vec<TermProbe> {
    let mut out = Vec::new();
    // 复用器三家：尺寸再大也不许开全屏/inline 路径。
    for mux in [MuxKind::Tmux, MuxKind::Screen, MuxKind::Zellij] {
        out.push(TermProbe {
            inside_mux: Some(mux),
            ..good_probe()
        });
    }
    // 尺寸下限（列, 行）的两侧与未报尺寸。
    for size in [(43, 12), (44, 11), (0, 0)] {
        out.push(TermProbe {
            size,
            ..good_probe()
        });
    }
    out.push(TermProbe {
        color: false,
        ..good_probe()
    });
    for term in [Some("dumb".to_string()), None, Some(String::new())] {
        out.push(TermProbe {
            term,
            ..good_probe()
        });
    }
    out.push(TermProbe {
        stdin_tty: false,
        ..good_probe()
    });
    out.push(TermProbe {
        stdout_tty: false,
        ..good_probe()
    });
    out
}

/// bug：显式 `--tui inline` 绕过探针门（非 TTY / 复用器里启 inline →
/// 手写 ANSI 打进管道或不支持的环境）。inline 的门 = enhanced 完全同款
/// （route §3 T8 设计注记 ①，tmux/Screen/Zellij → linear 照旧）。
#[test]
fn explicit_inline_uses_the_same_gate_as_enhanced() {
    // 门全过 + 显式请求 → 第四档。
    assert_eq!(
        resolve_mode(TuiMode::Inline, &good_probe()),
        TuiMode::Inline
    );
    // 门不齐的每一种：inline 一律回落 linear。
    for probe in ineligible_probes() {
        assert!(!enhanced_eligible(&probe), "门判据漂了：{probe:?}");
        assert_eq!(
            resolve_mode(TuiMode::Inline, &probe),
            TuiMode::Linear,
            "inline 门不齐必须回落 linear：{probe:?}"
        );
    }
    // `--tui linear` 请求早退照旧。
    assert_eq!(
        resolve_mode(TuiMode::Linear, &good_probe()),
        TuiMode::Linear
    );
}

/// bug：auto 档自己选出 inline（auto 判定语义零回归是 T8 的硬前提）——
/// 第四档只由显式请求进入，auto 在整个判据矩阵上只落 enhanced / linear。
#[test]
fn auto_never_selects_inline() {
    let mut probes = vec![good_probe()];
    probes.extend(ineligible_probes());
    for probe in &probes {
        let mode = resolve_mode(TuiMode::Auto, probe);
        assert!(
            matches!(mode, TuiMode::Enhanced | TuiMode::Linear),
            "auto 永不选 inline：{probe:?} → {mode:?}"
        );
    }
    // 门全过时 auto 仍裁决 enhanced（现状零回归）。
    assert_eq!(
        resolve_mode(TuiMode::Auto, &good_probe()),
        TuiMode::Enhanced
    );
}

/// CLI 解析（handoff §4 ⑧）：`--tui` 进第四臂、非法值仍报错——
/// 与 `bin/kymic/src/cli.rs` 同款 `value_enum` 参数形态。
#[test]
fn cli_parses_inline_as_the_fourth_arm_and_rejects_junk() {
    use clap::{Parser, ValueEnum};

    #[derive(Parser, Debug)]
    struct TuiCli {
        #[arg(long = "tui", value_enum, default_value = "auto")]
        mode: TuiMode,
    }

    // 四臂齐：auto | enhanced | inline | linear。
    assert_eq!(TuiMode::value_variants().len(), 4);
    for (text, want) in [
        ("auto", TuiMode::Auto),
        ("enhanced", TuiMode::Enhanced),
        ("inline", TuiMode::Inline),
        ("linear", TuiMode::Linear),
    ] {
        let cli =
            TuiCli::try_parse_from(["oi", "--tui", text]).unwrap_or_else(|e| panic!("{text}: {e}"));
        assert_eq!(cli.mode, want, "--tui {text}");
    }
    // 默认臂仍是 auto。
    let default = TuiCli::try_parse_from(["oi"]).expect("默认值可解析");
    assert_eq!(default.mode, TuiMode::Auto);
    // 非法值照旧报错（非法档位不许静默回落）。
    let err = TuiCli::try_parse_from(["oi", "--tui", "nonsense"]).expect_err("非法值必须报错");
    assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue);
}
