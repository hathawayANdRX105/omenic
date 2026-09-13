//! markdown_to_html 的 ammonia 清洗测试：透传的原始 HTML 必须被剥离，
//! 正常 markdown 渲染不受影响（输出经 dangerous_inner_html 注入 DOM）。

use omenic_web_components::chat::markdown_to_html;

#[test]
fn raw_script_tag_is_stripped() {
    let out = markdown_to_html("<script>alert(1)</script>hi");
    assert!(!out.contains("<script"), "script 标签被透传: {out}");
    // 正文文本仍在
    assert!(out.contains("hi"), "正常文本丢失: {out}");
}

#[test]
fn inline_event_attributes_are_stripped() {
    let out = markdown_to_html("<img src=\"x\" onerror=\"alert(1)\">");
    assert!(!out.contains("onerror"), "事件属性被透传: {out}");
}

#[test]
fn javascript_url_in_link_is_stripped() {
    let out = markdown_to_html("[点我](javascript:alert(1))");
    assert!(
        !out.to_lowercase().contains("javascript:"),
        "javascript: URL 被透传: {out}"
    );
}

#[test]
fn normal_markdown_still_renders() {
    let out = markdown_to_html("## 标题\n\n**加粗** 与 `code`");
    assert!(out.contains("<h2>"), "标题丢失: {out}");
    assert!(out.contains("<strong>"), "加粗丢失: {out}");
    assert!(out.contains("<code>"), "行内代码丢失: {out}");
}
