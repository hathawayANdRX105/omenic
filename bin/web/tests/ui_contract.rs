//! Web UI 契约测试（specs/ui/*.yaml，ROADMAP C5.10 前置）。
//!
//! 把已通过用户验收的 dsh 风格视觉/结构锁进契约：spec 用「DOM/源码锚点」
//! 而非像素。两类断言：
//!
//! a) 契约完整性 —— specs/ui 下 7 个 yaml 必须能被 serde_yaml 解析，
//!    且含 name / target / anchors（非空）；每个锚点带 find / expect /
//!    source 字段。
//! b) 源码锚点抽查 —— 每个锚点的 find 关键字（稳定的 class 片段或静态
//!    字面量）必须出现在其 omenic 实现文件中（spec 顶层 target，或锚点级
//!    file 覆盖）。刻意避开数据接线代码（并行 5.2a 分支正在改逻辑段），
//!    只锚样式与静态结构。
//!
//! spec 字段约定见 specs/ui/*.yaml 头注释与 AGENTS.md「Web UI 契约验收」。

use serde_yaml::Value;
use std::path::{Path, PathBuf};

/// 契约锁定的 7 个 spec 文件名（新 spec 必须同步登记到 SPEC_FILES）。
const SPEC_FILES: &[&str] = &[
    "workspace.yaml",
    "chat.yaml",
    "sidebar.yaml",
    "stats.yaml",
    "settings.yaml",
    "quick-switcher.yaml",
    "taskpanel.yaml",
];

/// 仓库根（bin/web/tests → 上三级）。
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../")
        .canonicalize()
        .expect("仓库根路径解析失败")
}

/// specs/ui 目录。
fn specs_dir() -> PathBuf {
    repo_root().join("specs/ui")
}

/// 解析单个 spec；失败时 panic 并带文件路径。
fn parse_spec(name: &str) -> Value {
    let path = specs_dir().join(name);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("读取 {} 失败: {e}", path.display()));
    serde_yaml::from_str(&raw).unwrap_or_else(|e| panic!("解析 {} 失败: {e}", path.display()))
}

/// 取 mapping 的字符串字段（缺失/类型不符即 panic）。
fn str_field<'a>(value: &'a Value, key: &str, ctx: &str) -> &'a str {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{ctx}: 缺少字符串字段 {key}"))
}

/// a) 契约完整性：7 个文件可被 serde_yaml 解析，name/target/anchors 非空，
///    且每个锚点都带非空 find / expect / source。
#[test]
fn ui_specs_are_complete_and_parseable() {
    for name in SPEC_FILES {
        let spec = parse_spec(name);
        assert!(
            !str_field(&spec, "name", name).is_empty(),
            "{name}: name 为空"
        );
        assert!(
            !str_field(&spec, "target", name).is_empty(),
            "{name}: target 为空"
        );
        let anchors = spec
            .get("anchors")
            .and_then(Value::as_sequence)
            .unwrap_or_else(|| panic!("{name}: anchors 缺失或不是列表"));
        assert!(!anchors.is_empty(), "{name}: anchors 为空");
        for (i, anchor) in anchors.iter().enumerate() {
            let ctx = format!("{name}: 第 {} 个锚点", i + 1);
            for required in ["find", "expect", "source"] {
                assert!(
                    !str_field(anchor, required, &ctx).is_empty(),
                    "{ctx} 的 {required} 为空"
                );
            }
        }
    }
}

/// specs/ui 里没有未登记的 yaml（防止加了契约却绕过抽查）。
#[test]
fn ui_specs_dir_has_no_unregistered_files() {
    let mut actual: Vec<String> = std::fs::read_dir(specs_dir())
        .expect("specs/ui 目录不存在")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|f| f.ends_with(".yaml"))
        .collect();
    actual.sort_unstable();
    let mut expected: Vec<String> = SPEC_FILES.iter().map(|s| (*s).to_string()).collect();
    expected.sort_unstable();
    assert_eq!(actual, expected, "specs/ui 与 SPEC_FILES 登记不一致");
}

/// b) 源码锚点抽查：find 关键字必须出现在对应 omenic 实现文件中。
///    文件解析顺序：锚点级 file 覆盖 → spec 顶层 target（相对仓库根）。
#[test]
fn anchors_exist_in_omenic_sources() {
    for name in SPEC_FILES {
        let spec = parse_spec(name);
        let default_target = str_field(&spec, "target", name).to_string();
        let anchors = spec
            .get("anchors")
            .and_then(Value::as_sequence)
            .unwrap_or_else(|| panic!("{name}: anchors 缺失"));
        assert!(!anchors.is_empty(), "{name}: 没有可抽查的锚点");
        for anchor in anchors {
            let find = str_field(anchor, "find", name).to_string();
            let file = anchor
                .get("file")
                .and_then(Value::as_str)
                .unwrap_or(&default_target);
            let path = repo_root().join(file);
            let src = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("读取实现文件 {} 失败: {e}", path.display()));
            assert!(
                src.contains(&find),
                "{name}: 锚点关键字 {find:?} 不在 {} 中——实现偏离契约，或关键字选了易变文本",
                path.display()
            );
        }
    }
}
