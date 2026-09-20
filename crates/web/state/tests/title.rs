//! title_from_first_message 的纯函数测试：截词 / 去标记 / 占位 / 字符计数。

use omenic_web_state::title_from_first_message;

#[test]
fn empty_input_falls_back_to_placeholder() {
    // bug 场景：占位逻辑被改成 panic、返回空串、或把 "仅空白" 当成有内容。
    assert_eq!(title_from_first_message(""), "新会话");
    assert_eq!(title_from_first_message("   "), "新会话");
    assert_eq!(title_from_first_message("\n\n\t"), "新会话");
    // 整行只剩 markdown 标记也不是可读标题。
    assert_eq!(title_from_first_message("###"), "新会话");
}

#[test]
fn truncates_to_40_chars() {
    // bug 场景：按 bytes 截断 → 中文字符被切半成乱码；或漏了省略号；
    // 或阈值取成 39/41。
    let long: String = "字".repeat(50);
    let got = title_from_first_message(&long);
    assert_eq!(got.chars().count(), 41, "40 字符 + 1 个省略号");
    assert!(got.ends_with('…'));
    assert!(!got.ends_with("字…字"));
    let inner: String = got.chars().take(40).collect();
    assert_eq!(inner, "字".repeat(40));
    // 恰好 40 字符：不截断、不加省略号（边界值）。
    let exact: String = "字".repeat(40);
    assert_eq!(title_from_first_message(&exact), exact);
}

#[test]
fn strips_markdown_prefix() {
    // bug 场景：忘了剥 `#`（标题以 "# " 开头）或取了整段多行正文。
    assert_eq!(title_from_first_message("# 标题\n正文"), "标题");
    assert_eq!(title_from_first_message("## 二级标题"), "二级标题");
    assert_eq!(title_from_first_message("- 列表项"), "列表项");
    assert_eq!(title_from_first_message("> 引用"), "引用");
    assert_eq!(title_from_first_message("**加粗**"), "加粗**");
    assert_eq!(title_from_first_message("  普通文本"), "普通文本");
    // 多行只取首个有内容的行（跳过空行）。
    assert_eq!(
        title_from_first_message("\n\n第二行才是\n第四行"),
        "第二行才是"
    );
}

#[test]
fn strips_compound_markdown_prefix() {
    // bug 场景：`trim_start_matches` 被换成单次剥刺（只吃掉一个标记字符），
    // 侧栏标题就带着 `> - ` / `## ` 前缀显示；或忘了 `\t` 也算行首空白。
    assert_eq!(title_from_first_message("> - 标题"), "标题");
    assert_eq!(title_from_first_message("## > - **粗**"), "粗**");
    // 钉 `'\t'` 在标记字符集合里：tab 出现在 `>` 之后（不是行首），
    // `body.trim()` 吃不到它，只有剥刺闭包认它才能落成 "标题"。会红：闭包里
    // 删掉 `'\t'`（标记字符后的 tab 不再被剥，标题变成 "\t标题"）。
    assert_eq!(title_from_first_message(">\t标题"), "标题");
}

#[test]
fn emoji_counts_as_chars() {
    // bug 场景：按 byte 数截断 → emoji（多字节）被切成半个替换字符。
    let emojis: String = "🎉".repeat(50);
    let got = title_from_first_message(&emojis);
    assert_eq!(got.chars().count(), 41);
    assert!(got.ends_with('…'));
    // emoji 与中文混合：各自占一个标量值，阈值仍是 40。
    let mixed: String = format!("{}{}", "中".repeat(20), "🚀".repeat(30));
    let got = title_from_first_message(&mixed);
    assert_eq!(got.chars().filter(|&c| c == '中').count(), 20);
    assert_eq!(got.chars().filter(|&c| c == '🚀').count(), 20);
    assert!(got.ends_with('…'));
    assert_eq!(got.chars().count(), 41);
}

#[test]
fn smoke_real_inputs_from_task_text() {
    // 真实输入样本（取自本任务书消息文本）：确认侧栏可读、长度受控。
    assert_eq!(
        title_from_first_message("帮我把这段 Rust 代码重构一下"),
        "帮我把这段 Rust 代码重构一下"
    );
    let title = title_from_first_message(
        "这是一条非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常非常长的用户消息",
    );
    assert_eq!(title.chars().count(), 41);
    assert!(title.ends_with('…'));
}
