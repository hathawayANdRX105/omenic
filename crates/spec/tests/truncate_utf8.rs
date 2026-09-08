use spec::shared::truncate_utf8;

/// Regression for the gh_wrap PR_MERGE log panic: `&reason[..min(80)]`
/// panicked when byte 80 landed inside a multi-byte CJK char (hit while
/// squash-merging PR #320, whose merge reason was Chinese). The truncation
/// helper must cut on char boundaries and never split a codepoint.
#[test]
fn truncate_utf8_never_splits_cjk() {
    // '死' occupies bytes 78..81 — exactly where the old slice cut.
    let reason = "规则路径修复 + daemon mut 恢复，检查三条规则与占位宏识别是否完整死";
    let cut = truncate_utf8(reason, 80);
    assert!(cut.is_char_boundary(cut.len()));
    assert!(reason.starts_with(cut));

    // Pure CJK shorter than the cap passes through unchanged.
    assert_eq!(truncate_utf8("中文", 80), "中文");
    // Empty input is safe.
    assert_eq!(truncate_utf8("", 80), "");
}
