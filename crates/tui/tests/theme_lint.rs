//! theme_lint — D11 的字面量禁令：`crates/tui/src` 全部源码不许出现裸
//! ESC 字节 / `ESC [` 文本转义 / 十六进制色值——样式只能从
//! `crate::theme` 的语义 token 出发（命名色，配色决定权留给终端调色板）。
//!
//! 本文件自己就含这些字符串（扫描规则），所以只扫 `src/`，不扫 tests/。

use std::path::{Path, PathBuf};

/// 扫描规则命中的违规清单为空即通过。
#[test]
fn source_contains_no_raw_escape_or_hex_literals() {
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let files = rust_files(&src_dir);
    assert!(
        files.len() >= 10,
        "expected a full recursive walk of src, found {} files in {src_dir:?}",
        files.len()
    );

    let mut violations = Vec::new();
    for file in &files {
        let name = file
            .strip_prefix(&src_dir)
            .unwrap_or(file)
            .display()
            .to_string();
        let bytes = std::fs::read(file).unwrap_or_else(|e| panic!("must read {name}: {e}"));
        // 1) 裸 ESC 字节（0x1B）直接进源码。
        if bytes.contains(&0x1b) {
            violations.push(format!("{name}: raw ESC byte (0x1b)"));
        }
        // 文本形态统一按小写扫：`\x1b` / `\u{1b}` / `\e[` 三种写法都是
        // 在源码里埋转义序列，渲染期会原样写终端。
        let text = String::from_utf8_lossy(&bytes).to_lowercase();
        for needle in ["\\x1b", "\\u{1b}", "\\e["] {
            if text.contains(needle) {
                violations.push(format!("{name}: escape literal `{needle}`"));
            }
        }
        // 2) `0x` 后跟十六进制数字：数值字面量（含 RGB 码）。
        if let Some(hit) = find_prefix_hex(&text, '0', 'x', 1) {
            violations.push(format!("{name}: hex number literal near `{hit}`"));
        }
        // 3) `#` 后跟连续 ≥6 个十六进制数字：CSS 形态色值。
        if let Some(hit) = find_hash_hex(&text, 6) {
            violations.push(format!("{name}: hex color literal near `{hit}`"));
        }
    }
    assert!(
        violations.is_empty(),
        "style literals must stay in theme.rs (named colors only):\n{}",
        violations.join("\n")
    );
}

/// 递归收集目录下全部 `.rs`（顺序无关，断言只看违规清单）。
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("must read dir {dir:?}: {e}"));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|e| panic!("dir entry in {dir:?}: {e}"))
            .path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
    out
}

/// 找 `<lead><trail>` 后跟至少 `min_hex` 个十六进制数字的片段（返回命中处
/// 的一小段上下文，便于报错定位）。按字节扫：`0` / `#` 都是 ASCII，UTF-8
/// 多字节序列的续字节不会撞上。
fn find_prefix_hex(text: &str, lead: char, trail: char, min_hex: usize) -> Option<String> {
    let bytes = text.as_bytes();
    let (l, t) = (lead as u8, trail as u8);
    for i in 0..bytes.len() {
        if bytes[i] != l || bytes.get(i + 1) != Some(&t) {
            continue;
        }
        let hex = bytes
            .iter()
            .skip(i + 2)
            .take(min_hex)
            .filter(|b| b.is_ascii_hexdigit())
            .count();
        if hex >= min_hex {
            return Some(snippet(text, i));
        }
    }
    None
}

/// 找 `#` 后跟连续 ≥`min_hex` 个十六进制数字（CSS 色值形态）。
fn find_hash_hex(text: &str, min_hex: usize) -> Option<String> {
    let bytes = text.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] != b'#' {
            continue;
        }
        let run = bytes
            .iter()
            .skip(i + 1)
            .take_while(|b| b.is_ascii_hexdigit())
            .count();
        if run >= min_hex {
            return Some(snippet(text, i));
        }
    }
    None
}

/// 命中位置起的一小段上下文（`at` 落在 ASCII 字节上，必是字符边界）。
fn snippet(text: &str, at: usize) -> String {
    text[at..].chars().take(24).collect()
}
